//! Object names for the rows on screen (#1896).
//!
//! The UI tells the browser which objects of a device it shows without a
//! name; that set replaces the device's [`NameQueue`], so a row scrolled
//! off before its turn is never read. A name job (the [`Scheduler`]'s
//! lowest priority, under the same per-device gate and [`RateCap`] as
//! every other read) waits for a free slot in the cap before it takes a
//! batch from the queue, so a row withdrawn while it waited costs neither
//! a request nor a slot. It reads the batch with one ReadPropertyMultiple
//! (Clause 15.7) or, from a device that refused it, one ReadProperty each
//! (Clause 15.5), and reports it.
//!
//! The queue remembers what is being read and, for a while, what was
//! answered ([`ANSWERS_KEPT`]), so a set the UI sends before an answer
//! reaches it doesn't read a name twice: a name asked for again is
//! reported again from memory. A name the device refused is not read
//! again until the device is read afresh. A device that stops answering
//! ends the job, and every name it held that the UI still wants is
//! reported unreachable; those are read again once the UI says the device
//! answered (an I-Am, [`NameQueue::retry`]).
//!
//! [`Scheduler`]: super::limits::Scheduler
//! [`RateCap`]: super::limits::RateCap

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::read::{rpm_failure, unreachable, Halt, Reader, RpmFailure};
use crate::tui::format::name_text;
use crate::tui::point::{ReadName, ReqId};

/// Names one ReadPropertyMultiple asks for. Small enough that the answer
/// usually fits an unsegmented APDU; when it doesn't, that batch is read
/// one name at a time.
pub(crate) const NAMES_PER_RPM: usize = 8;

/// Answers a device's queue remembers, the oldest forgotten first (#1931).
/// They cover answers still on their way to the UI when it asks again, and
/// a name the UI let go of and asks for again. A forgotten answer is read
/// again if it is asked for.
pub(crate) const ANSWERS_KEPT: usize = 256;

/// Rows of one ask the queue takes; a screen shows far fewer.
pub(crate) const MAX_WANTED: usize = 1_024;

/// One object and how its name read went.
pub(crate) type NameRead = (ObjectIdentifier, ReadName);

/// One device's names still to read.
#[derive(Debug, Default)]
pub(crate) struct NameQueue {
    /// The request that set `wanted`, for the reports.
    req: ReqId,
    /// The device's address when the UI asked, echoed in the reports so
    /// the UI files each name under the device it asked about.
    address: String,
    /// Names to read, in the order the UI shows them.
    wanted: VecDeque<ObjectIdentifier>,
    /// Those of the batch being read that the UI still wants. A row
    /// withdrawn while its batch is out leaves it, so it is neither read
    /// by ReadProperty nor reported unreachable.
    in_flight: Vec<ObjectIdentifier>,
    /// How each name read went, unreachable ones too until the device
    /// answers again; at most [`ANSWERS_KEPT`].
    answered: HashMap<ObjectIdentifier, ReadName>,
    /// The keys of `answered`, oldest first.
    order: VecDeque<ObjectIdentifier>,
    /// Answers asked for again, to report without a request.
    replay: Vec<NameRead>,
}

impl NameQueue {
    /// Replace the names to read with `objects` (the first [`MAX_WANTED`]),
    /// less those being read; those already answered are reported again
    /// from memory. Another `address` means another device behind the
    /// instance, so every answer is forgotten. Returns whether there is
    /// anything to read or report.
    pub(crate) fn want(
        &mut self,
        req: ReqId,
        address: String,
        objects: Vec<ObjectIdentifier>,
    ) -> bool {
        if address != self.address {
            self.forget();
            self.in_flight.clear();
            self.address = address;
        }
        self.req = req;
        self.wanted.clear();
        let objects = &objects[..objects.len().min(MAX_WANTED)];
        self.in_flight.retain(|object| objects.contains(object));
        for &object in objects {
            if self.in_flight.contains(&object) || self.wanted.contains(&object) {
                continue;
            }
            match self.answered.get(&object) {
                Some(read) if !self.replay.iter().any(|(o, _)| *o == object) => {
                    self.replay.push((object, read.clone()));
                }
                Some(_) => {}
                None => self.wanted.push_back(object),
            }
        }
        self.waiting()
    }

    /// True while names wait to be read or reported.
    pub(crate) fn waiting(&self) -> bool {
        !self.wanted.is_empty() || !self.replay.is_empty()
    }

    /// True while the queue holds nothing for the UI: no name to read or
    /// report, and no batch out.
    pub(crate) fn idle(&self) -> bool {
        !self.waiting() && self.in_flight.is_empty()
    }

    /// The device answered again: names it didn't answer for may be read.
    pub(crate) fn retry(&mut self) {
        self.answered
            .retain(|_, read| *read != ReadName::Unreachable);
        let answered = &self.answered;
        self.order.retain(|object| answered.contains_key(object));
    }

    /// Forget every answer, so names the device refused may be asked for
    /// again (the device is being read afresh).
    pub(crate) fn forget(&mut self) {
        self.answered.clear();
        self.order.clear();
    }

    /// The answers to report again, with the request and address to
    /// report them under.
    fn take_replay(&mut self) -> (ReqId, String, Vec<NameRead>) {
        let replay = std::mem::take(&mut self.replay);
        (self.req, self.address.clone(), replay)
    }

    /// The next batch of up to `count`, now in flight, with the request
    /// and address to report it under.
    fn take(&mut self, count: usize) -> (ReqId, String, Vec<ObjectIdentifier>) {
        let take = count.min(self.wanted.len());
        self.in_flight = self.wanted.drain(..take).collect();
        (self.req, self.address.clone(), self.in_flight.clone())
    }

    /// True while the UI still wants `object` of the batch in flight.
    fn still_wanted(&self, object: ObjectIdentifier) -> bool {
        self.in_flight.contains(&object)
    }

    fn record(&mut self, read: &[NameRead]) {
        for (object, outcome) in read {
            if self.answered.insert(*object, outcome.clone()).is_none() {
                self.order.push_back(*object);
            }
        }
        while self.answered.len() > ANSWERS_KEPT {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.answered.remove(&oldest);
        }
    }

    /// The batch in flight ended with `read` answered. The rest of it, all
    /// still wanted, goes back to the front of the queue; a row asked for
    /// again while the batch was out, and answered by it, isn't read twice.
    fn settle(&mut self, read: &[NameRead]) {
        self.record(read);
        for object in std::mem::take(&mut self.in_flight).into_iter().rev() {
            if !self.answered.contains_key(&object) && !self.wanted.contains(&object) {
                self.wanted.push_front(object);
            }
        }
        let answered = &self.answered;
        self.wanted.retain(|object| !answered.contains_key(object));
    }

    /// The device stopped answering after `read`: everything else the UI
    /// still wants, the batch in flight and the whole queue, is given up
    /// on as unreachable and returned. Rows withdrawn meanwhile are not.
    fn abandon(&mut self, read: &[NameRead]) -> Vec<ObjectIdentifier> {
        self.record(read);
        let held: Vec<ObjectIdentifier> = std::mem::take(&mut self.in_flight)
            .into_iter()
            .chain(self.wanted.drain(..))
            .collect();
        let mut left: Vec<ObjectIdentifier> = Vec::new();
        for object in held {
            if !self.answered.contains_key(&object) && !left.contains(&object) {
                left.push(object);
            }
        }
        let gone: Vec<NameRead> = left.iter().map(|&o| (o, ReadName::Unreachable)).collect();
        self.record(&gone);
        left
    }
}

fn lock(queue: &Mutex<NameQueue>) -> std::sync::MutexGuard<'_, NameQueue> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Read names from `queue` until it is empty or the job is stopped,
/// handing each batch, and each answer reported again from memory, to
/// `report` with the request and the device address it was asked under.
/// The lock is held only to take and settle a batch, never across a
/// request or a wait.
///
/// No overshoot: nothing may await between [`Reader::free_turn`] and the
/// request's slot (`acquire` in [`Reader::turn`]). Taking the batch
/// between them is synchronous, so the slot the free turn found is the
/// one the request books, and a row withdrawn during the wait is never
/// read. An await there (an async lock in `take`, say) would let other
/// jobs book that slot, and this batch would wait a slot more for rows
/// that may be gone by then.
pub(crate) async fn read_names<T, F, R>(
    reader: &mut Reader<'_, T>,
    queue: &Mutex<NameQueue>,
    mut report: F,
) where
    T: TransportPort + 'static,
    F: FnMut(ReqId, String, Vec<NameRead>) -> R,
    R: std::future::Future<Output = ()>,
{
    loop {
        let (req, address, replay) = lock(queue).take_replay();
        if !replay.is_empty() {
            report(req, address, replay).await;
        }
        if lock(queue).wanted.is_empty() {
            return;
        }
        // A free slot first, then the batch, with no await between them.
        if reader.free_turn().await.is_err() {
            return;
        }
        let per_batch = if reader.rpm { NAMES_PER_RPM } else { 1 };
        let (req, address, batch) = lock(queue).take(per_batch);
        if batch.is_empty() {
            return;
        }
        let (mut read, halt) = read_batch(reader, queue, &batch).await;
        let stopped = halt.is_some();
        match halt {
            None | Some(Halt::Cancelled) => lock(queue).settle(&read),
            Some(Halt::Device(error)) => {
                tracing::info!(
                    "device {}: names not read ({error}); asking again when it next answers",
                    reader.device
                );
                let left = lock(queue).abandon(&read);
                read.extend(left.into_iter().map(|o| (o, ReadName::Unreachable)));
            }
        }
        if !read.is_empty() {
            report(req, address, read).await;
        }
        if stopped {
            return;
        }
    }
}

/// One batch: by RPM when the device takes it and the batch has more than
/// one name, else by ReadProperty each; either way only the names the UI
/// still wants when the request goes. Returns what was answered, and why
/// the batch stopped early: the job was stopped, or the device isn't
/// answering.
async fn read_batch<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    queue: &Mutex<NameQueue>,
    batch: &[ObjectIdentifier],
) -> (Vec<NameRead>, Option<Halt>) {
    if reader.rpm && batch.len() > 1 {
        match rpm_names(reader, queue, batch).await {
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
        if !lock(queue).still_wanted(object) {
            continue;
        }
        match reader
            .rp(object, PropertyIdentifier::OBJECT_NAME, None)
            .await
        {
            Ok(data) => read.push((object, named(name_text(&data)))),
            Err(Halt::Device(error)) if unreachable(&error) => {
                return (read, Some(Halt::Device(error)))
            }
            Err(Halt::Device(_)) => read.push((object, ReadName::Refused)),
            Err(Halt::Cancelled) => return (read, Some(Halt::Cancelled)),
        }
    }
    (read, None)
}

/// A decoded name, or a refusal when the answer wasn't one string.
fn named(name: Option<String>) -> ReadName {
    name.map_or(ReadName::Refused, ReadName::Named)
}

/// Object_Name of the objects in `batch` the UI still wants once the
/// request's slot comes, with one RPM. An object the answer leaves out,
/// or answers with an error, has no name.
async fn rpm_names<T: TransportPort + 'static>(
    reader: &Reader<'_, T>,
    queue: &Mutex<NameQueue>,
    batch: &[ObjectIdentifier],
) -> Result<Vec<NameRead>, Halt> {
    reader.turn().await?;
    let batch: Vec<ObjectIdentifier> = {
        let queue = lock(queue);
        batch
            .iter()
            .copied()
            .filter(|object| queue.still_wanted(*object))
            .collect()
    };
    if batch.is_empty() {
        return Ok(Vec::new());
    }
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
            (object, named(name))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;

    use super::*;

    const HERE: &str = "10.0.0.7:47808";

    fn ai(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
    }

    fn one(name: &str) -> ReadName {
        ReadName::Named(name.into())
    }

    fn want(queue: &mut NameQueue, req: ReqId, objects: Vec<ObjectIdentifier>) -> bool {
        queue.want(req, HERE.into(), objects)
    }

    /// A new set replaces the old one, skipping what is in flight; what
    /// was answered is reported again rather than read; a stopped batch
    /// goes back to the front while still wanted.
    #[test]
    fn the_queue_keeps_only_what_the_screen_still_wants() {
        let mut queue = NameQueue::default();
        assert!(want(&mut queue, 1, (1..=10).map(ai).collect()));
        let (req, address, batch) = queue.take(3);
        assert_eq!((req, address.as_str()), (1, HERE));
        assert_eq!(batch, vec![ai(1), ai(2), ai(3)]);
        // The screen moved: 1 and 2 are in flight but 3 scrolled off; 20
        // is new, and 4-10 went.
        assert!(want(&mut queue, 2, vec![ai(1), ai(2), ai(20)]));
        assert_eq!(queue.wanted, [ai(20)]);
        assert!(!queue.still_wanted(ai(3)));
        // Stopped after 1 was read: 2 returns to the front, 3 doesn't.
        queue.settle(&[(ai(1), one("one"))]);
        assert_eq!(queue.wanted, [ai(2), ai(20)]);
        // Answered already: reported again, not read.
        assert!(want(&mut queue, 3, vec![ai(1)]));
        assert!(queue.wanted.is_empty());
        assert_eq!(queue.take_replay().2, [(ai(1), one("one"))]);
        assert!(!queue.waiting());
        // A device that stops answering gives up on everything held, as
        // unreachable: read again only after `retry`.
        want(&mut queue, 4, vec![ai(5), ai(6)]);
        queue.take(1);
        assert_eq!(queue.abandon(&[]), [ai(5), ai(6)]);
        want(&mut queue, 5, vec![ai(5), ai(6)]);
        assert!(queue.wanted.is_empty(), "replayed as unreachable");
        queue.take_replay();
        queue.retry();
        assert!(
            want(&mut queue, 6, vec![ai(5)]),
            "asked again after the device answered"
        );
        assert_eq!(queue.wanted, [ai(5)]);
        // A refusal stays until the device is read afresh.
        queue.take(1);
        queue.settle(&[(ai(5), ReadName::Refused)]);
        queue.retry();
        want(&mut queue, 7, vec![ai(5)]);
        assert!(queue.wanted.is_empty());
        queue.take_replay();
        queue.forget();
        assert!(want(&mut queue, 8, vec![ai(5)]));
        assert_eq!(queue.wanted, [ai(5)]);
    }

    /// A name withdrawn while in flight and then asked for again is still
    /// wanted: a stopped batch puts it back, and a device that stops
    /// answering reports it.
    #[test]
    fn a_name_asked_again_while_in_flight_is_not_lost() {
        for stopped_by_timeout in [false, true] {
            let mut queue = NameQueue::default();
            want(&mut queue, 1, vec![ai(1)]);
            queue.take(1);
            assert!(!want(&mut queue, 2, Vec::new()));
            assert!(want(&mut queue, 3, vec![ai(1)]), "wanted again");
            if stopped_by_timeout {
                assert_eq!(queue.abandon(&[]), [ai(1)]);
            } else {
                queue.settle(&[]);
                assert_eq!(queue.wanted, [ai(1)]);
            }
        }
    }

    /// Rows withdrawn while their batch is out are neither read on nor
    /// given up as unreachable when the device stops answering (#1931).
    #[test]
    fn withdrawn_rows_are_not_marked_unreachable() {
        let mut queue = NameQueue::default();
        want(&mut queue, 1, vec![ai(1), ai(2), ai(3)]);
        queue.take(3);
        assert!(!want(&mut queue, 2, vec![ai(2)]));
        assert!(!queue.still_wanted(ai(1)) && queue.still_wanted(ai(2)));
        assert_eq!(queue.abandon(&[]), [ai(2)]);
        assert!(!queue.answered.contains_key(&ai(1)));
        assert!(!queue.answered.contains_key(&ai(3)));
        // And a batch that ends is put back only as far as still wanted.
        want(&mut queue, 3, vec![ai(4), ai(5)]);
        queue.take(2);
        want(&mut queue, 4, vec![ai(5)]);
        queue.settle(&[]);
        assert_eq!(queue.wanted, [ai(5)]);
    }

    /// The queue remembers at most [`ANSWERS_KEPT`] answers, forgetting
    /// the oldest; one forgotten is read again when asked for. Another
    /// address behind the instance forgets them all.
    #[test]
    fn answers_are_bounded_and_kept_per_address() {
        let mut queue = NameQueue::default();
        let count = u32::try_from(ANSWERS_KEPT).unwrap() + 10;
        for instance in 0..count {
            want(&mut queue, 1, vec![ai(instance)]);
            queue.take(1);
            queue.settle(&[(ai(instance), one("n"))]);
        }
        assert_eq!(queue.answered.len(), ANSWERS_KEPT);
        assert_eq!(queue.order.len(), ANSWERS_KEPT);
        want(&mut queue, 2, vec![ai(0), ai(count - 1)]);
        assert_eq!(queue.wanted, [ai(0)], "the oldest was forgotten");
        assert_eq!(queue.replay.len(), 1);
        // The same instance at another address: nothing is known there.
        queue.take_replay();
        queue.want(3, "10.0.9.7:47808".into(), vec![ai(count - 1)]);
        assert_eq!(queue.wanted, [ai(count - 1)]);
        assert!(queue.answered.is_empty());
        // An ask never takes more than MAX_WANTED rows.
        let many = (0..u32::try_from(MAX_WANTED).unwrap() * 2)
            .map(ai)
            .collect();
        want(&mut queue, 4, many);
        assert_eq!(queue.wanted.len(), MAX_WANTED);
    }
}
