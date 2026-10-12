//! MS/TP (Master-Slave/Token-Passing) transport over RS-485 (Clause 9).
//!
//! The master node state machine is the portable core in
//! [`bacnet_mstp_core`], and frames are encoded and decoded by
//! [`bacnet_mstp_codec`] (re-exported here as [`codec`]). This module adds
//! what a host needs around them:
//!
//! - [`SerialPort`], the serial I/O the transport runs over: `tokio-serial`,
//!   kernel RS-485 and GPIO direction control in
//!   [`mstp_serial`](crate::mstp_serial), and [`LoopbackSerial`] and
//!   [`NoSerial`] for tests and non-MS/TP builds.
//! - [`MstpTransport`], whose Tokio task drives the core: it feeds every
//!   received octet to the core, services the core's deadlines with Tokio
//!   time, and carries out the core's actions (frames to send, NPDUs to
//!   deliver). The `adapter` module documents the details.
//! - [`MstpConfig`], mapped onto [`MstpCoreConfig`](bacnet_mstp_core::MstpCoreConfig)
//!   and validated by the core's rules.
//! - [`MstpDiagnostics`], the core's counters plus counts kept at the
//!   transport boundary.

use bacnet_mstp_core::{ConfigError, Duration as CoreDuration, MstpCoreConfig};
use bacnet_types::error::Error;
use tokio::sync::oneshot;

pub use bacnet_mstp_codec as codec;
pub use bacnet_mstp_core::{MstpCounters, QueueCounters};

mod adapter;
mod diagnostics;
mod queues;
pub use diagnostics::{MstpDiagnostics, MstpDiagnosticsSnapshot};

// ---------------------------------------------------------------------------
// Serial port abstraction
// ---------------------------------------------------------------------------

/// What a serial port's receiver hears while the port itself transmits.
///
/// RS-485 is half duplex. Whether a station hears its own frames depends on
/// how the transceiver is wired: with the receiver enable tied to the driver
/// enable (the usual MAX485 hookup) the receiver is off while the driver is
/// on, and some drivers and USB adapters filter their own output, so
/// nothing comes back. A transceiver whose receiver stays on returns every
/// octet the port sends.
///
/// The transport must know which: the core counts what it hears after a
/// transmission as line activity, and its own Token heard back would look
/// like the successor using the token. With [`Echo::Full`] the transport
/// drops as many received octets as it sent, before it reports the
/// transmission complete to the core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Echo {
    /// Nothing the port sends comes back to its receiver.
    #[default]
    None,
    /// Every octet the port sends comes back to its receiver, in order.
    Full,
}

/// Abstraction over an RS-485 serial port.
///
/// Implementations wrap the platform serial driver (e.g. `tokio-serial`).
/// [`LoopbackSerial`] is an in-memory implementation for tests.
///
/// The transport reads and writes from one task, never at the same time, and
/// may drop a pending [`read`](Self::read) future (to service a deadline, or
/// to look for octets that are already waiting just before it transmits), so
/// `read` must be cancel safe: a dropped read must lose no data.
pub trait SerialPort: Send + Sync + 'static {
    /// Write bytes to the serial port. Success may mean only driver acceptance,
    /// not that the final stop bit has left the UART; use [`Self::drain`] for that.
    ///
    /// The transport writes one whole frame per call, and only once the core
    /// has confirmed the frame may start: a port that controls the
    /// transceiver direction itself enables the line driver here.
    fn write(&self, data: &[u8]) -> impl std::future::Future<Output = Result<(), Error>> + Send;
    /// Wait until all accepted output has finished transmission, including the
    /// UART shift register. Required for software-controlled RS-485 direction.
    ///
    /// Call even after a write error, which may have accepted a partial frame.
    /// An error leaves transmit completion unknown. The default is unsupported,
    /// rather than falsely reporting completion for existing custom backends;
    /// the transport then takes the frame's time on the wire at the configured
    /// baud rate as its completion.
    fn drain(&self) -> impl std::future::Future<Output = Result<(), Error>> + Send {
        async {
            Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Serial transmit-complete drain is not supported by this backend",
            )))
        }
    }
    /// Read available bytes into `buf`. Returns the number of bytes read.
    /// Should block until at least 1 byte is available or timeout.
    fn read(
        &self,
        buf: &mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, Error>> + Send;
    /// What the port's receiver hears while the port transmits; see [`Echo`].
    /// The default is [`Echo::None`].
    fn echo(&self) -> Echo {
        Echo::None
    }
}

/// Execution placement for an MS/TP transport, independent of master-node settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MstpExecutionMode {
    /// Spawn the MAC loop on the caller's Tokio runtime (the existing default).
    #[default]
    Tokio,
    /// Poll the same MAC loop on a dedicated OS thread with its own Tokio runtime.
    /// Blocking drain work uses that runtime's blocking pool, not the application's.
    /// This does not configure real-time scheduling or guarantee wire timing.
    DedicatedThread,
}

/// Owns the isolated reactor. Dropping the shutdown sender also handles cancelled
/// startup and abort; only `stop` waits for runtime and blocking-I/O completion.
struct DedicatedRuntime {
    handle: tokio::runtime::Handle,
    shutdown: oneshot::Sender<()>,
    finished: oneshot::Receiver<()>,
}

impl DedicatedRuntime {
    async fn new() -> Result<Self, Error> {
        let (started_tx, started_rx) = oneshot::channel();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let (finished_tx, finished) = oneshot::channel();
        std::thread::Builder::new()
            .name("bacnet-mstp".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        if started_tx.send(Ok(runtime.handle().clone())).is_ok() {
                            let _ = runtime.block_on(shutdown_rx);
                        }
                        // Drop on the OS thread, never in the caller's async context.
                        // This waits for any already-started blocking drain calls.
                        drop(runtime);
                    }
                    Err(error) => {
                        let _ = started_tx.send(Err(error));
                    }
                }
                let _ = finished_tx.send(());
            })
            .map_err(Error::Transport)?;
        let handle = started_rx
            .await
            .map_err(|_| Error::Encoding("MS/TP execution thread failed to start".into()))?
            .map_err(Error::Transport)?;
        Ok(Self {
            handle,
            shutdown,
            finished,
        })
    }

    async fn stop(self) {
        drop(self.shutdown);
        let _ = self.finished.await;
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// T_frame_abort for a host serial port (Clause 9.5.3 allows up to 100 ms).
///
/// A host reads the line in chunks, and a USB adapter can hold a frame's
/// tail back for several milliseconds (16 ms by default on FTDI parts), so
/// the 60 bit times a UART-driven station uses would abort frames that are
/// still arriving. The core times each octet from the read that returned it.
pub const HOST_FRAME_ABORT_MS: u64 = 100;

/// Master node configuration.
///
/// [`MstpTransport::start`](crate::port::TransportPort::start) maps it onto
/// the core's [`MstpCoreConfig`] and refuses a configuration the core
/// refuses; [`Self::validate`] runs the same checks without starting. Every
/// Clause 9 timer the core uses comes from the baud rate and the fixed
/// Clause 9.5.3 values, except T_frame_abort (see [`HOST_FRAME_ABORT_MS`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MstpConfig {
    /// This station's MAC address (0..=max_master).
    pub this_station: u8,
    /// Highest master address on the link, Max_Master (at most 127).
    pub max_master: u8,
    /// Max_Info_Frames: information frames per token use (1..=255).
    pub max_info_frames: u8,
    /// Baud rate, from which the bit-time timers (T_turnaround and the
    /// rest) are derived (default: 9600). The core needs at least 2667 baud,
    /// so that 40 bit times of turnaround fit inside T_usage_delay.
    pub baud_rate: u32,
}

impl Default for MstpConfig {
    fn default() -> Self {
        Self {
            this_station: 0,
            max_master: codec::MAX_MASTER,
            max_info_frames: 1,
            baud_rate: 9600,
        }
    }
}

impl MstpConfig {
    /// Check the configuration as [`start`](crate::port::TransportPort::start)
    /// does, without opening anything.
    ///
    /// # Errors
    ///
    /// [`Error::Encoding`] naming the field: `max_master` above 127,
    /// `this_station` above `max_master`, `max_info_frames` of zero, or a
    /// `baud_rate` of zero or too low for Clause 9 timing.
    pub fn validate(&self) -> Result<(), Error> {
        self.core_config().map(drop)
    }

    /// The core configuration this one maps onto.
    pub(crate) fn core_config(&self) -> Result<MstpCoreConfig, Error> {
        let mut config = MstpCoreConfig::for_baud_rate(self.baud_rate)
            .map_err(|error| self.config_error(error))?;
        config.station = self.this_station;
        config.max_master = self.max_master;
        config.max_info_frames = self.max_info_frames;
        config.t_frame_abort = CoreDuration::from_millis(HOST_FRAME_ABORT_MS);
        config
            .validate()
            .map_err(|error| self.config_error(error))?;
        Ok(config)
    }

    fn config_error(&self, error: ConfigError) -> Error {
        let message = match error {
            ConfigError::MaxMasterOutOfRange { value } => {
                format!("MS/TP max_master {value} exceeds MAX_MASTER (127)")
            }
            ConfigError::StationExceedsMaxMaster {
                station,
                max_master,
            } => {
                format!("MS/TP this_station {station} exceeds configured max_master ({max_master})")
            }
            ConfigError::MaxInfoFramesOutOfRange { .. } => {
                "MS/TP max_info_frames must be in 1..=255".to_string()
            }
            ConfigError::ZeroBaudRate => "MS/TP baud_rate must not be zero".to_string(),
            other => format!(
                "MS/TP baud_rate {} is outside what Clause 9 timing allows ({other:?})",
                self.baud_rate
            ),
        };
        Error::Encoding(message)
    }
}

mod port;
pub use port::{LoopbackSerial, MstpTransport, NoSerial};

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod diagnostics_tests;
#[cfg(test)]
mod echo_tests;
#[cfg(test)]
pub(crate) mod execution_tests;
#[cfg(test)]
mod reply_tests;
#[cfg(test)]
mod tests;
