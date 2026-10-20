//! The device browser's reads.
//!
//! [`run`] takes browse commands from the session loop and runs each as a
//! job: a sequence of confirmed requests to one device, made one after
//! another. The [`Scheduler`] lets a device run one job at a time, and a
//! [`RateCap`] spaces every request across all jobs. Results carry the
//! request id the UI chose, so it drops a result it no longer wants.
//! Object names for the rows on screen are read by [`names`], through the
//! same scheduler and cap.
//!
//! The session polls this loop with `join!`, not as a `select!` arm, so a
//! job waiting inside the client is never left unpolled while the session
//! loop awaits something of its own. The loop's own arms never await.

pub(crate) mod limits;
pub(crate) mod names;
pub(crate) mod read;
pub(crate) mod view;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;

use self::limits::{Job, RateCap, Scheduler, Started, Task};
use self::names::NameQueue;
use self::read::{Halt, Reader, Wanted};
use super::EventSink;
use crate::tui::message::{Command, WorkerEvent};
use crate::tui::point::{BrowseRead, BrowseResult, NameAsk, ReqId};

/// Default `--max-rate`: confirmed requests a second across all devices.
pub(crate) const DEFAULT_MAX_RATE: u32 = 20;

/// Object_List elements between progress reports.
const PROGRESS_EVERY: u32 = 25;

/// The Device properties on the card (Clause 12.11).
const CARD: [PropertyIdentifier; 13] = [
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::VENDOR_NAME,
    PropertyIdentifier::VENDOR_IDENTIFIER,
    PropertyIdentifier::MODEL_NAME,
    PropertyIdentifier::FIRMWARE_REVISION,
    PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
    PropertyIdentifier::PROTOCOL_VERSION,
    PropertyIdentifier::PROTOCOL_REVISION,
    PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
    PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED,
    PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
    PropertyIdentifier::SEGMENTATION_SUPPORTED,
    PropertyIdentifier::DATABASE_REVISION,
];

/// What a finished job tells the loop.
struct Done {
    device: u32,
    /// The device refused RPM during this job.
    refused_rpm: bool,
}

type JobFuture<'a> = Pin<Box<dyn Future<Output = Done> + Send + 'a>>;

/// Each device's name queue, shared with its name job.
type Queues = HashMap<u32, Arc<Mutex<NameQueue>>>;

/// Run `f` on `device`'s name queue, if it has one.
fn with_queue<R>(queues: &Queues, device: u32, f: impl FnOnce(&mut NameQueue) -> R) -> Option<R> {
    let queue = queues.get(&device)?;
    Some(f(&mut queue.lock().unwrap_or_else(PoisonError::into_inner)))
}

/// Serve browse commands until `commands` closes, every request after a
/// slot from `rate`, the cap the watch list shares. Dropping the returned
/// future drops every job, and with it any request still outstanding.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<Command>,
    rate: &RateCap,
) {
    let mut scheduler = Scheduler::default();
    // Devices that refused RPM go straight to ReadProperty next time.
    let mut no_rpm = HashSet::new();
    let mut queues = Queues::new();
    let mut jobs: FuturesUnordered<JobFuture<'_>> = FuturesUnordered::new();
    let start = |started: Started, no_rpm: &HashSet<u32>, queues: &Queues| -> JobFuture<'_> {
        let rpm = !no_rpm.contains(&started.device);
        let queue = queues.get(&started.device).cloned().unwrap_or_default();
        Box::pin(job(client, sink, rate, started, rpm, queue))
    };
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Names { req, asks }) => {
                    for NameAsk { device, objects, retry } in asks {
                        queues.entry(device).or_default();
                        let waiting = with_queue(&queues, device, |queue| {
                            if retry {
                                queue.retry();
                            }
                            queue.want(req, objects)
                        });
                        if waiting == Some(true) {
                            if let Some(started) = scheduler.submit_names(device) {
                                jobs.push(start(started, &no_rpm, &queues));
                            }
                        }
                    }
                }
                Some(Command::Browse { req, device, read }) => {
                    // A fresh read of the device may ask again for names
                    // it couldn't give before.
                    if read == BrowseRead::Device {
                        with_queue(&queues, device, NameQueue::forget);
                    }
                    if let Some(started) = scheduler.submit(device, Job { req, read }) {
                        jobs.push(start(started, &no_rpm, &queues));
                    }
                }
                Some(Command::StopBrowse { device }) => scheduler.stop(device),
                Some(_) => {}
                None => break,
            },
            Some(done) = jobs.next(), if !jobs.is_empty() => {
                if done.refused_rpm {
                    no_rpm.insert(done.device);
                }
                let names = with_queue(&queues, done.device, |queue| queue.waiting()).unwrap_or(false);
                if let Some(started) = scheduler.finished(done.device, names) {
                    jobs.push(start(started, &no_rpm, &queues));
                }
            }
        }
    }
}

async fn job<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    rate: &RateCap,
    started: Started,
    rpm: bool,
    queue: Arc<Mutex<NameQueue>>,
) -> Done {
    let Started {
        device,
        task,
        cancel,
    } = started;
    let mut reader = Reader {
        client,
        device,
        rate,
        cancel: &cancel,
        rpm,
    };
    let (req, read) = match task {
        Task::Read(Job { req, read }) => (req, read),
        Task::Names => {
            let report = |req, names| deliver(sink, req, BrowseResult::Names { device, names });
            names::read_names(&mut reader, &queue, report).await;
            return Done {
                device,
                refused_rpm: rpm && !reader.rpm,
            };
        }
    };
    let outcome = match read {
        BrowseRead::Device => read_device(&mut reader, sink, req).await,
        BrowseRead::Object(object) => read_object(&mut reader, sink, req, object).await,
    };
    match outcome {
        Ok(()) | Err(Halt::Cancelled) => {}
        Err(Halt::Device(error)) => {
            deliver(sink, req, BrowseResult::Failed(error.to_string())).await;
        }
    }
    Done {
        device,
        refused_rpm: rpm && !reader.rpm,
    }
}

async fn deliver(sink: &EventSink, req: ReqId, result: BrowseResult) {
    sink.deliver(WorkerEvent::Browse { req, result }).await;
}

/// The device card, then the Object_List.
async fn read_device<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    sink: &EventSink,
    req: ReqId,
) -> Result<(), Halt> {
    let device = reader.device_object()?;
    let (rows, via) = reader.properties(device, Wanted::List(&CARD)).await?;
    deliver(sink, req, BrowseResult::Card(view::card(&rows, via))).await;
    let read = reader
        .object_list(|done, total| {
            if done.is_multiple_of(PROGRESS_EVERY) {
                // Progress may be dropped when the UI falls behind.
                sink.offer(WorkerEvent::Browse {
                    req,
                    result: BrowseResult::Progress { done, total },
                });
            }
        })
        .await?;
    deliver(
        sink,
        req,
        BrowseResult::Objects {
            objects: read.objects,
            indexed: read.indexed,
            stopped: read.stopped,
        },
    )
    .await;
    Ok(())
}

async fn read_object<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    sink: &EventSink,
    req: ReqId,
    object: ObjectIdentifier,
) -> Result<(), Halt> {
    let (rows, via) = reader.properties(object, Wanted::All).await?;
    let view = view::object_view(object, &rows, via);
    deliver(sink, req, BrowseResult::Object(Box::new(view))).await;
    Ok(())
}
