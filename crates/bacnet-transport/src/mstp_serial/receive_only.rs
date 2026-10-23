//! Native read-only serial ownership for passive monitoring.
use super::SerialConfig;
#[cfg(unix)]
use crate::mstp::monitor::Failure;
use crate::mstp::monitor::MonitorRead;
use bacnet_types::error::Error;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

#[cfg(any(windows, test))]
mod threaded;
#[cfg(unix)]
use crate::sys::serial_receive_unix::Input;
#[cfg(unix)]
use tokio::io::unix::AsyncFd;

/// A serial input opened with native read-only access and exclusive ownership.
///
/// This type has no write interface, configures no kernel RS-485 mode and
/// requests no GPIO. Opening/configuring a driver can still change modem lines;
/// native read-only access does not guarantee electrical silence. An explicit
/// RTS level is applied after open, so it cannot prevent an open-time transition.
///
/// Await [`close`](Self::close), or the monitor's stop, before reopening the
/// adapter. Drop requests shutdown but is not a completion barrier on Windows.
#[derive(Debug)]
pub struct ReceiveOnlyPort {
    #[cfg(unix)]
    input: Option<AsyncFd<Input>>,
    #[cfg(unix)]
    closed: Option<Result<(), Failure>>,
    #[cfg(windows)]
    input: threaded::Reader,
}
impl ReceiveOnlyPort {
    /// Open exclusive raw binary 8N1 input at the requested baud, purging input.
    ///
    /// `request_to_send` optionally sets RTS after configuration. No rate is
    /// silently substituted. macOS custom rates use IOSSIOSPEED, whose actual
    /// physical rate cannot be read back through termios.
    ///
    /// # Errors
    /// Returns native open, exclusivity, configuration or purge failures.
    pub fn open(config: &SerialConfig, request_to_send: Option<bool>) -> Result<Self, Error> {
        Self::open_native(config, request_to_send).map_err(|error| {
            Error::Transport(crate::mstp::monitor::context(
                error,
                format!(
                    "receive-only serial {} at {} baud",
                    config.port_name, config.baud_rate
                ),
            ))
        })
    }
    #[cfg(unix)]
    fn open_native(config: &SerialConfig, rts: Option<bool>) -> io::Result<Self> {
        let input = Input::open(&config.port_name, config.baud_rate, rts)?;
        let input = match AsyncFd::try_with_interest(input, tokio::io::Interest::READABLE) {
            Ok(input) => input,
            Err(error) => {
                let (mut input, error) = error.into_parts();
                return crate::mstp::monitor::combine(Err(error), input.close())
                    .map(|()| unreachable!());
            }
        };
        Ok(Self {
            input: Some(input),
            closed: None,
        })
    }
    #[cfg(windows)]
    fn open_native(config: &SerialConfig, rts: Option<bool>) -> io::Result<Self> {
        let input = crate::sys::serial_receive_windows::Input::open(
            &config.port_name,
            config.baud_rate,
            rts,
        )?;
        Ok(Self {
            input: threaded::Reader::start(input)?,
        })
    }
    /// Stop reading and await native release, retaining progress if cancelled.
    ///
    /// # Errors
    /// Reports read, close and reader-thread failures. A repeated call retains
    /// a previous failure. A driver that never completes a read can delay close.
    pub async fn close(&mut self) -> io::Result<()> {
        std::future::poll_fn(|cx| self.poll_close(cx)).await
    }
}
impl AsyncRead for ReceiveOnlyPort {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        #[cfg(windows)]
        {
            Pin::new(&mut self.input).poll_read(cx, buf)
        }
        #[cfg(unix)]
        {
            use std::io::Read;
            if buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            let Some(input) = self.input.as_mut() else {
                return Poll::Ready(Ok(()));
            };
            loop {
                let mut ready = std::task::ready!(input.poll_read_ready_mut(cx))?;
                match ready.try_io(|input| input.get_mut().read(buf.initialize_unfilled())) {
                    Ok(Ok(count)) => {
                        buf.advance(count);
                        return Poll::Ready(Ok(()));
                    }
                    Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Ok(Err(error)) => return Poll::Ready(Err(error)),
                    Err(_) => continue,
                }
            }
        }
    }
}
impl MonitorRead for ReceiveOnlyPort {
    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        #[cfg(windows)]
        {
            self.input.poll_close(_cx)
        }
        #[cfg(unix)]
        {
            if self.closed.is_none() {
                let result = self
                    .input
                    .take()
                    .map_or(Ok(()), |input| input.into_inner().close());
                self.closed = Some(result.map_err(Failure::new));
            }
            Poll::Ready(
                self.closed
                    .as_ref()
                    .expect("closed result")
                    .as_ref()
                    .copied()
                    .map_err(Failure::error),
            )
        }
    }
}

#[cfg(windows)]
impl threaded::Input for crate::sys::serial_receive_windows::Input {
    fn close(&mut self) -> io::Result<()> {
        self.close()
    }
}
#[cfg(all(test, unix))]
mod unix_tests;
