//! Object names for the rows on screen (#1896).
//!
//! The UI tells the browser which objects of a device it shows without a
//! name; that set replaces the device's [`NameQueue`], so a row scrolled
//! off before its turn is never read. A name job (the [`Scheduler`]'s
//! lowest priority, under the same per-device gate and [`RateCap`] as
//! every other read) takes a few names at a time from the queue, reads
//! them with one ReadPropertyMultiple (Clause 15.7) or, from a device that
//! refused it, one ReadProperty each (Clause 15.5), and reports them.
//!
//! The queue remembers what is being read and what was answered, so a set
//! the UI sends before an answer reaches it doesn't read a name twice. A
//! name the device can't give is reported as `None`, which the UI keeps
//! for the session instead of asking again. A device that stops answering
//! ends the job with every name it still held reported that way.
//!
//! [`Scheduler`]: super::limits::Scheduler
//! [`RateCap`]: super::limits::RateCap

use std::collections::{HashSet, VecDeque};
use std::sync::{Mutex, PoisonError};

use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::read::{rpm_failure, unreachable, Halt, Reader, RpmFailure};
use crate::tui::format::name_text;
use crate::tui::point::ReqId;

/// Names one ReadPropertyMultiple asks for. Small enough that the answer
/// usually fits an unsegmented APDU; when it doesn't, that batch is read
/// one name at a time.
pub(crate) const NAMES_PER_RPM: usize = 8;

/// One object's name as read: `None` when the device couldn't give it.
pub(crate) type NameRead = (ObjectIdentifier, Option<String>);

/// One device's names still to read.
#[derive(Debug, Default)]
pub(crate) struct NameQueue {
    /// The request that set `wanted`, for the reports.
    req: ReqId,
    /// Names to read, in the order the UI shows them.
    wanted: VecDeque<ObjectIdentifier>,
    /// The batch being read now.
    in_flight: Vec<ObjectIdentifier>,
    /// Those of `in_flight` the UI still wants: a stopped batch puts them
    /// back in the queue.
    keep: Vec<ObjectIdentifier>,
    /// Names answered this session, read or not.
    answered: HashSet<ObjectIdentifier>,
}

impl NameQueue {
    /// Replace the names to read with `objects`, less those being read or
    /// already answered. Returns whether any are left to read.
    pub(crate) fn want(&mut self, req: ReqId, objects: Vec<ObjectIdentifier>) -> bool {
        self.req = req;
        self.wanted.clear();
        self.keep.retain(|object| objects.contains(object));
        for object in objects {
            if !self.in_flight.contains(&object)
                && !self.answered.contains(&object)
                && !self.wanted.contains(&object)
            {
                self.wanted.push_back(object);
            }
        }
        !self.wanted.is_empty()
    }

    /// True while names wait to be read.
    pub(crate) fn waiting(&self) -> bool {
        !self.wanted.is_empty()
    }

    /// Forget what was answered, so names the device couldn't give may be
    /// asked for again (the device is being read afresh).
    pub(crate) fn forget(&mut self) {
        self.answered.clear();
    }

    /// The next batch of up to `count`, now in flight.
    fn take(&mut self, count: usize) -> (ReqId, Vec<ObjectIdentifier>) {
        let take = count.min(self.wanted.len());
        self.in_flight = self.wanted.drain(..take).collect();
        self.keep.clone_from(&self.in_flight);
        (self.req, self.in_flight.clone())
    }

    /// The batch in flight ended with `read` answered. The rest of it goes
    /// back to the front of the queue if the UI still wants it.
    fn settle(&mut self, read: &[NameRead]) {
        self.in_flight.clear();
        self.answered.extend(read.iter().map(|(object, _)| *object));
        for object in std::mem::take(&mut self.keep).into_iter().rev() {
            if !self.answered.contains(&object) && !self.wanted.contains(&object) {
                self.wanted.push_front(object);
            }
        }
    }

    /// Everything still held, given up on: the batch in flight not yet
    /// answered and the whole queue.
    fn abandon(&mut self) -> Vec<ObjectIdentifier> {
        self.keep.clear();
        let mut left: Vec<ObjectIdentifier> = std::mem::take(&mut self.in_flight)
            .into_iter()
            .chain(self.wanted.drain(..))
            .filter(|object| !self.answered.contains(object))
            .collect();
        left.dedup();
        self.answered.extend(left.iter().copied());
        left
    }
}

fn lock(queue: &Mutex<NameQueue>) -> std::sync::MutexGuard<'_, NameQueue> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Read names from `queue` until it is empty or the job is stopped,
/// handing each batch to `report` as it is answered. The lock is held only
/// to take and settle a batch, never across a request.
pub(crate) async fn read_names<T, F, R>(
    reader: &mut Reader<'_, T>,
    queue: &Mutex<NameQueue>,
    mut report: F,
) where
    T: TransportPort + 'static,
    F: FnMut(ReqId, Vec<NameRead>) -> R,
    R: std::future::Future<Output = ()>,
{
    loop {
        let per_batch = if reader.rpm { NAMES_PER_RPM } else { 1 };
        let (req, batch) = lock(queue).take(per_batch);
        if batch.is_empty() {
            return;
        }
        let (mut read, halt) = read_batch(reader, &batch).await;
        let stopped = halt.is_some();
        match halt {
            None | Some(Halt::Cancelled) => lock(queue).settle(&read),
            Some(Halt::Device(error)) => {
                tracing::info!(
                    "device {}: names not read ({error}); not asking it again this session",
                    reader.device
                );
                let mut queue = lock(queue);
                queue.settle(&read);
                read.extend(queue.abandon().into_iter().map(|object| (object, None)));
            }
        }
        if !read.is_empty() {
            report(req, read).await;
        }
        if stopped {
            return;
        }
    }
}

/// One batch: by RPM when the device takes it and the batch has more than
/// one name, else by ReadProperty each. Returns what was answered, and why
/// the batch stopped early: the job was stopped, or the device isn't
/// answering.
async fn read_batch<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    batch: &[ObjectIdentifier],
) -> (Vec<NameRead>, Option<Halt>) {
    if reader.rpm && batch.len() > 1 {
        match rpm_names(reader, batch).await {
            Ok(read) => return (read, None),
            Err(Halt::Cancelled) => return (Vec::new(), Some(Halt::Cancelled)),
            Err(Halt::Device(error)) => match rpm_failure(&error) {
                RpmFailure::Refused => {
                    tracing::info!(
                        "device {} refused ReadPropertyMultiple ({error}); reading names one at a time",
                        reader.device
                    );
                    reader.rpm = false;
                }
                RpmFailure::Other if unreachable(&error) => {
                    return (Vec::new(), Some(Halt::Device(error)))
                }
                // Too large, or an error for the whole request that one
                // object may have caused: each name on its own says which.
                RpmFailure::TooLarge | RpmFailure::Other => {}
            },
        }
    }
    let mut read = Vec::with_capacity(batch.len());
    for &object in batch {
        match reader
            .rp(object, PropertyIdentifier::OBJECT_NAME, None)
            .await
        {
            Ok(data) => read.push((object, name_text(&data))),
            Err(Halt::Device(error)) if unreachable(&error) => {
                return (read, Some(Halt::Device(error)))
            }
            Err(Halt::Device(_)) => read.push((object, None)),
            Err(Halt::Cancelled) => return (read, Some(Halt::Cancelled)),
        }
    }
    (read, None)
}

/// Object_Name of every object in `batch` with one RPM. An object the
/// answer leaves out, or answers with an error, has no name.
async fn rpm_names<T: TransportPort + 'static>(
    reader: &Reader<'_, T>,
    batch: &[ObjectIdentifier],
) -> Result<Vec<NameRead>, Halt> {
    let specs = batch
        .iter()
        .map(|&object| ReadAccessSpecification {
            object_identifier: object,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
            }],
        })
        .collect();
    reader.turn().await?;
    let ack = reader
        .client
        .read_property_multiple_from_device(reader.device, specs)
        .await
        .map_err(Halt::Device)?;
    Ok(batch
        .iter()
        .map(|&object| {
            let name = ack
                .list_of_read_access_results
                .iter()
                .filter(|result| result.object_identifier == object)
                .flat_map(|result| &result.list_of_results)
                .find(|element| element.property_identifier == PropertyIdentifier::OBJECT_NAME)
                .and_then(|element| element.property_value.as_deref())
                .and_then(name_text);
            (object, name)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;

    use super::*;

    fn ai(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
    }

    /// A new set replaces the old one, skipping what is in flight or was
    /// answered; a stopped batch goes back to the front while still wanted.
    #[test]
    fn the_queue_keeps_only_what_the_screen_still_wants() {
        let mut queue = NameQueue::default();
        assert!(queue.want(1, (1..=10).map(ai).collect()));
        let (req, batch) = queue.take(3);
        assert_eq!((req, batch.clone()), (1, vec![ai(1), ai(2), ai(3)]));
        // The screen moved: 1 and 2 are in flight but 3 scrolled off; 20
        // is new, and 4-10 went.
        assert!(queue.want(2, vec![ai(1), ai(2), ai(20)]));
        assert_eq!(queue.wanted, [ai(20)]);
        // Stopped after 1 was read: 2 returns to the front, 3 doesn't.
        queue.settle(&[(ai(1), Some("one".into()))]);
        assert_eq!(queue.wanted, [ai(2), ai(20)]);
        assert!(!queue.want(3, vec![ai(1)]), "answered already");
        // A device that stops answering gives up on everything held.
        queue.want(4, vec![ai(5), ai(6)]);
        queue.take(1);
        assert_eq!(queue.abandon(), [ai(5), ai(6)]);
        assert!(!queue.want(5, vec![ai(5), ai(6)]));
        queue.forget();
        assert!(queue.want(6, vec![ai(5)]), "asked again after forget");
    }
}
