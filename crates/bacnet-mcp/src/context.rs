//! What every tool call shares: the engine, the policy, the cursors and
//! caches ([`Ctx`]), and one call's budget and cost ([`Call`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_client::client::DiscoveredRouter;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_ops::engine::Engine;
use bacnet_ops::limits::{Account, TokenBucket};
use bacnet_ops::target::{self, Target};
use bacnet_transport::port::TransportPort;
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::cursor::Cursors;
use crate::errors::ToolError;
use crate::policy::Policy;

/// The server's state, shared by every call.
pub(crate) struct Ctx<T: TransportPort + 'static> {
    /// The engine; `None` in passive mode, where no client is built.
    pub(crate) engine: Option<Arc<Engine<T>>>,
    pub(crate) policy: Policy,
    /// The one principal of a stdio server: the process's OS user.
    pub(crate) principal: String,
    pub(crate) cursors: Cursors,
    /// The principal's token bucket.
    pub(crate) bucket: Arc<TokenBucket>,
    /// When the last broad Who-Is went.
    pub(crate) broad: Mutex<Option<Instant>>,
    /// Object lists read, by device, for paging without reading again.
    pub(crate) lists: Mutex<HashMap<u32, CachedList>>,
    /// One lock a device, held while its list is read, so two calls on one
    /// device take turns and the second goes on from where the first
    /// stopped.
    pub(crate) list_locks: Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>,
    /// Calls and requests per device, logged as a summary at shutdown.
    pub(crate) reads: Mutex<BTreeMap<u32, (u64, u64)>>,
    /// Object names and descriptions read by `bacnet_find_object`, by
    /// device, so its later pages don't read them again.
    pub(crate) names: Mutex<HashMap<u32, CachedNames>>,
    /// What Who-Is-Router-To-Network queries found, and when the last one
    /// went; at most one goes a minute.
    pub(crate) routers: Mutex<Routers>,
}

/// The routers known from Who-Is-Router-To-Network queries.
#[derive(Debug, Clone, Default)]
pub(crate) struct Routers {
    /// When the last query went: a claim taken before it is sent, which
    /// holds off the next for a minute whether or not it finished.
    pub(crate) asked: Option<Instant>,
    /// When the last query that finished went, and the routers it heard
    /// (at most [`ROUTER_CAP`]).
    pub(crate) heard: Option<(Instant, Vec<DiscoveredRouter>)>,
}

/// The most routers kept, the size of a router table.
pub(crate) const ROUTER_CAP: usize = 256;

/// One device's object names and descriptions as read.
#[derive(Debug, Clone)]
pub(crate) struct CachedNames {
    /// When the first was read; the whole entry goes after [`LIST_TTL`].
    pub(crate) at: Instant,
    pub(crate) objects: HashMap<ObjectIdentifier, Names>,
}

/// An object's name and description, kept already cleaned and capped
/// ([`crate::untrusted::clean`]): the raw device text is never stored.
#[derive(Debug, Clone, Default)]
pub(crate) struct Names {
    pub(crate) name: Option<Cleaned>,
    /// `None` until read; then the description, if the device gave one.
    pub(crate) description: Option<Option<Cleaned>>,
}

/// Device text after [`crate::untrusted::clean`], and whether the cap cut it.
#[derive(Debug, Clone)]
pub(crate) struct Cleaned {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

/// A device's Object_List as read so far.
#[derive(Debug, Clone)]
pub(crate) struct CachedList {
    /// When it was read whole, or, for one read element by element, when
    /// that read began (not when it last went on).
    pub(crate) at: Instant,
    /// The entries read, in device order.
    pub(crate) objects: Arc<Vec<ObjectIdentifier>>,
    /// Read element by element: the element count, and the next element to
    /// read (past `total` once complete).
    pub(crate) indexed: Option<(u32, u32)>,
    /// Elements the device answered with an error, skipped.
    pub(crate) gaps: Vec<u32>,
}

impl CachedList {
    /// Whether every entry has been read.
    pub(crate) fn complete(&self) -> bool {
        self.indexed.is_none_or(|(total, next)| next > total)
    }

    /// Whether it may still be used: a complete list for [`LIST_TTL`]
    /// after it was read, a partial one for [`PARTIAL_TTL`] after its read
    /// began.
    pub(crate) fn fresh(&self) -> bool {
        let ttl = if self.complete() {
            LIST_TTL
        } else {
            PARTIAL_TTL
        };
        self.at.elapsed() < ttl
    }
}

/// How long an object list read for one page serves the next ones.
pub(crate) const LIST_TTL: Duration = Duration::from_secs(300);

/// How long an element-by-element read may go on across calls before it
/// starts again.
pub(crate) const PARTIAL_TTL: Duration = Duration::from_secs(600);

/// The most devices whose lists are kept; past it the oldest goes.
pub(crate) const LIST_CAP: usize = 64;

/// One tool call's budget and tally.
pub(crate) struct Call {
    pub(crate) started: Instant,
    /// Past this, a call that pages returns what it has.
    pub(crate) deadline: Instant,
    pub(crate) account: Arc<Account>,
    timeouts: AtomicU32,
    /// The device the call reached, for the read log.
    device: Mutex<Option<u32>>,
}

impl Call {
    pub(crate) fn new(budget: Duration, account: Arc<Account>) -> Self {
        let started = Instant::now();
        Self {
            started,
            deadline: started + budget,
            account,
            timeouts: AtomicU32::new(0),
            device: Mutex::new(None),
        }
    }

    /// True once the call's budget is spent.
    pub(crate) fn out_of_time(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Count a request that timed out.
    pub(crate) fn timed_out(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn reached(&self, device: u32) {
        *self.device.lock().unwrap_or_else(PoisonError::into_inner) = Some(device);
    }

    pub(crate) fn device(&self) -> Option<u32> {
        *self.device.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `cost` block every result carries.
    pub(crate) fn cost(&self) -> Value {
        let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        json!({
            "requests": self.account.requests(),
            "elapsed_ms": ms(self.started.elapsed()),
            "throttled_ms": ms(self.account.throttled()),
            "timeouts": self.timeouts.load(Ordering::Relaxed),
        })
    }
}

/// A device argument: an instance, `dnet:instance` or `dnet:0xMAC`, as a
/// string, or an instance as a number.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum DeviceArg {
    Number(u32),
    Text(String),
}

impl DeviceArg {
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Number(n) => n.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

/// A device the policy lets the call reach, from the discovery table.
pub(crate) struct Device {
    pub(crate) instance: u32,
    /// Its network, 0 for the local one.
    pub(crate) network: u16,
    pub(crate) entry: DiscoveredDevice,
}

const DISCOVER_HINT: &str = "call bacnet_discover first (with a range or network that includes it)";

impl<T: TransportPort + 'static> Ctx<T> {
    /// The engine, or a refusal in passive mode.
    pub(crate) fn engine(&self) -> Result<&Engine<T>, ToolError> {
        self.engine.as_deref().ok_or_else(|| {
            ToolError::refused(
                "passive",
                "the server is passive: it builds no client and sends nothing",
            )
        })
    }

    /// The device `arg` names, if the discovery table holds it and the
    /// policy allows it. Targets are numeric: a device is never found by
    /// name, and an address is no device.
    pub(crate) async fn device(&self, call: &Call, arg: &DeviceArg) -> Result<Device, ToolError> {
        let engine = self.engine()?;
        let text = arg.text();
        let unknown = || ToolError::UnknownDevice {
            device: text.clone(),
            hint: DISCOVER_HINT.into(),
        };
        let parsed = target::parse_target(&text).map_err(|e| ToolError::invalid("device", e))?;
        let entry = match parsed {
            Target::Instance(instance) => engine.device(instance).await.ok_or_else(unknown)?,
            Target::Routed { dnet, instance } => engine
                .device(instance)
                .await
                .filter(|found| found.source_network == Some(dnet))
                .ok_or_else(unknown)?,
            Target::RoutedMac { dnet, mac } => engine
                .devices()
                .await
                .into_iter()
                .find(|found| {
                    found.source_network == Some(dnet)
                        && found.source_address.as_deref() == Some(&mac[..])
                })
                .ok_or_else(unknown)?,
            Target::Mac(_) | Target::Bip6(_) => {
                return Err(ToolError::invalid(
                    "device",
                    "a device instance, dnet:instance or dnet:0xMAC, not an address",
                ));
            }
        };
        let device = Device {
            instance: entry.object_identifier.instance_number(),
            network: entry.source_network.unwrap_or(0),
            entry,
        };
        self.allow(device.instance, device.network)?;
        call.reached(device.instance);
        Ok(device)
    }

    /// Refuse a network the policy leaves out.
    pub(crate) fn allow_network(&self, network: u16) -> Result<(), ToolError> {
        if self.policy.allows_network(network) {
            return Ok(());
        }
        Err(ToolError::refused(
            "network-not-allowed",
            format!("network {network} is not in this server's [mcp] networks"),
        ))
    }

    /// Refuse a device or network the policy leaves out.
    pub(crate) fn allow(&self, instance: u32, network: u16) -> Result<(), ToolError> {
        self.allow_network(network)?;
        if !self.policy.allows_device(instance) {
            return Err(ToolError::refused(
                "device-not-allowed",
                format!("device {instance} is not in this server's [mcp] devices"),
            ));
        }
        Ok(())
    }

    /// Record a finished call in the read log, as a summary line.
    pub(crate) fn log_read(&self, tool: &str, call: &Call, outcome: &str) {
        let requests = call.account.requests();
        let device = call.device();
        tracing::info!(
            target: "bacnet_mcp::reads",
            tool,
            device,
            requests,
            outcome,
            "read call"
        );
        if let Some(device) = device {
            let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = reads.entry(device).or_default();
            entry.0 += 1;
            entry.1 += u64::from(requests);
        }
    }

    /// Log the read counts per device, at shutdown.
    pub(crate) fn log_summary(&self) {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        for (device, (calls, requests)) in reads.iter() {
            tracing::info!(
                target: "bacnet_mcp::reads",
                principal = %self.principal,
                device,
                calls,
                requests,
                "read summary"
            );
        }
    }

    /// The `limits` block every result carries.
    pub(crate) fn limits(&self, page: Option<usize>) -> Value {
        let rates = &self.policy.rates;
        let mut limits = json!({
            "rate_per_s": rates.global,
            "principal_rate_per_s": rates.principal,
            "per_device_outstanding": 1,
        });
        if let Some(page) = page {
            limits["page"] = json!(page);
        }
        limits
    }
}
