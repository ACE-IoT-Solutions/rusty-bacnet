use std::collections::VecDeque;
use std::time::Instant;

use crate::{ErrorCode, RuntimeError};

/// Runtime work priority independent of one transport's wire encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkPriority {
    /// Interactive commands and writes.
    Foreground,
    /// Due polling reads.
    Poll,
    /// Discovery and topology work.
    Discovery,
    /// Bulk scans and maintenance.
    Background,
}

/// Monotonic operation identity used for cancellation and observability.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId(pub u64);

/// Result of requesting cancellation for one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancelReport {
    /// Requested identity.
    pub id: OperationId,
    /// Operation was removed before dispatch.
    pub queued: bool,
    /// Cancellation was signalled to an executing operation.
    pub in_flight: bool,
}

impl CancelReport {
    /// Whether queued or in-flight work was found.
    pub fn found(&self) -> bool {
        self.queued || self.in_flight
    }
}

/// One accepted scheduler item.
#[derive(Debug)]
pub struct Scheduled<T> {
    /// Stable operation identity.
    pub id: OperationId,
    /// Runtime lane.
    pub priority: WorkPriority,
    /// Absolute deadline checked before dispatch.
    pub deadline: Instant,
    /// Caller payload.
    pub payload: T,
}

/// Result of selecting the next item.
#[derive(Debug)]
pub enum Dispatch<T> {
    /// Item may execute before its deadline.
    Ready(Scheduled<T>),
    /// Item expired in the queue and must not execute.
    Expired(Scheduled<T>),
}

/// Bounded four-lane scheduler with FIFO lanes and starvation prevention.
#[derive(Debug)]
pub struct WorkScheduler<T> {
    queues: [VecDeque<Scheduled<T>>; 4],
    capacity: usize,
    max_foreground_burst: usize,
    foreground_burst: usize,
    next_id: u64,
}

impl<T> WorkScheduler<T> {
    /// Creates a scheduler with one aggregate capacity and a foreground burst bound.
    pub fn new(capacity: usize, max_foreground_burst: usize) -> Result<Self, RuntimeError> {
        if capacity == 0 || max_foreground_burst == 0 {
            return Err(RuntimeError::invalid_config(
                "scheduler capacity and max_foreground_burst must be non-zero",
            ));
        }
        Ok(Self {
            queues: std::array::from_fn(|_| VecDeque::new()),
            capacity,
            max_foreground_burst,
            foreground_burst: 0,
            next_id: 1,
        })
    }

    /// Enqueues without waiting; saturation is explicit backpressure.
    pub fn enqueue(
        &mut self,
        priority: WorkPriority,
        deadline: Instant,
        payload: T,
    ) -> Result<OperationId, RuntimeError> {
        if self.len() >= self.capacity {
            return Err(RuntimeError {
                code: ErrorCode::Backpressure,
                retryable: true,
                attachment_id: None,
                message: format!("runtime scheduler capacity {} is full", self.capacity),
            });
        }
        let id = OperationId(self.next_id);
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.queues[lane(priority)].push_back(Scheduled {
            id,
            priority,
            deadline,
            payload,
        });
        Ok(id)
    }

    /// Removes queued work. In-flight cancellation is handled by its task token.
    pub fn cancel(&mut self, id: OperationId) -> Option<Scheduled<T>> {
        for queue in &mut self.queues {
            if let Some(position) = queue.iter().position(|item| item.id == id) {
                return queue.remove(position);
            }
        }
        None
    }

    /// Selects the next item, forcing a waiting lower lane after a foreground burst.
    pub fn next(&mut self, now: Instant) -> Option<Dispatch<T>> {
        let lower_waiting = self.queues[1..].iter().any(|queue| !queue.is_empty());
        let selected = if !self.queues[0].is_empty()
            && (!lower_waiting || self.foreground_burst < self.max_foreground_burst)
        {
            self.foreground_burst += 1;
            0
        } else {
            self.foreground_burst = 0;
            (1..4)
                .find(|index| !self.queues[*index].is_empty())
                .unwrap_or(0)
        };
        let item = self.queues[selected].pop_front()?;
        if now >= item.deadline {
            Some(Dispatch::Expired(item))
        } else {
            Some(Dispatch::Ready(item))
        }
    }

    /// Current queued work count.
    pub fn len(&self) -> usize {
        self.queues.iter().map(VecDeque::len).sum()
    }

    /// Whether no work is queued.
    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(VecDeque::is_empty)
    }

    /// Removes and returns every queued item.
    pub fn drain(&mut self) -> Vec<Scheduled<T>> {
        self.foreground_burst = 0;
        self.queues
            .iter_mut()
            .flat_map(|queue| queue.drain(..))
            .collect()
    }
}

fn lane(priority: WorkPriority) -> usize {
    match priority {
        WorkPriority::Foreground => 0,
        WorkPriority::Poll => 1,
        WorkPriority::Discovery => 2,
        WorkPriority::Background => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::ErrorCode;

    use super::{Dispatch, WorkPriority, WorkScheduler};

    #[test]
    fn bounded_queue_reports_backpressure_and_supports_cancellation() {
        let now = std::time::Instant::now();
        let mut scheduler = WorkScheduler::new(2, 2).unwrap();
        let first = scheduler
            .enqueue(WorkPriority::Background, now + Duration::from_secs(1), 1)
            .unwrap();
        scheduler
            .enqueue(WorkPriority::Foreground, now + Duration::from_secs(1), 2)
            .unwrap();
        assert_eq!(
            scheduler
                .enqueue(WorkPriority::Poll, now + Duration::from_secs(1), 3)
                .unwrap_err()
                .code,
            ErrorCode::Backpressure
        );
        assert_eq!(scheduler.cancel(first).unwrap().payload, 1);
        assert!(scheduler.cancel(first).is_none());
    }

    #[test]
    fn foreground_preempts_but_cannot_starve_lower_lanes() {
        let now = std::time::Instant::now();
        let deadline = now + Duration::from_secs(1);
        let mut scheduler = WorkScheduler::new(8, 2).unwrap();
        scheduler
            .enqueue(WorkPriority::Background, deadline, "background")
            .unwrap();
        for _ in 0..4 {
            scheduler
                .enqueue(WorkPriority::Foreground, deadline, "foreground")
                .unwrap();
        }
        let payloads = (0..3)
            .map(|_| match scheduler.next(now).unwrap() {
                Dispatch::Ready(item) => item.payload,
                Dispatch::Expired(_) => panic!("unexpected expiry"),
            })
            .collect::<Vec<_>>();
        assert_eq!(payloads, vec!["foreground", "foreground", "background"]);
    }

    #[test]
    fn elapsed_deadline_is_returned_expired_without_execution() {
        let now = std::time::Instant::now();
        let mut scheduler = WorkScheduler::new(1, 1).unwrap();
        scheduler
            .enqueue(WorkPriority::Poll, now - Duration::from_millis(1), 7)
            .unwrap();
        assert!(matches!(scheduler.next(now), Some(Dispatch::Expired(item)) if item.payload == 7));
        assert!(scheduler.is_empty());
    }
}
