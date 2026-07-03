//! A job queue behind a single [`Queue`] trait so workers and the API depend on one interface. This
//! crate ships the in-memory [`MemQueue`] that makes the service runnable now; the Phase-2 Postgres
//! implementation (a `SELECT … FOR UPDATE SKIP LOCKED` lease loop) slots in behind the same trait.
//!
//! Semantics that hold regardless of backend:
//!  - **Lease, don't dequeue.** [`Queue::lease`] hands a job to a worker but keeps ownership until it is
//!    [`ack`](Queue::ack)ed, so a crashed worker's job is not lost.
//!  - **Bounded retries with a dead-letter.** [`Queue::nack`] requeues a job until `max_attempts`, after
//!    which it moves to the dead-letter list instead of retrying forever.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

/// Opaque job identifier (matches `kerf-store`'s `JobId`).
pub type JobId = u64;

/// Outcome of a [`Queue::nack`]: either the job was requeued for another attempt, or it exhausted its
/// retries and was dead-lettered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nacked {
    Requeued { attempt: u32 },
    DeadLettered,
}

/// A durable job queue.
pub trait Queue: Send + Sync {
    /// Add a job to the back of the queue.
    fn enqueue(&self, job: JobId);
    /// Lease up to `max` queued jobs to a worker (removes them from the pending queue, but the queue
    /// retains ownership until `ack`/`nack`). Increments each job's attempt counter.
    fn lease(&self, max: usize) -> Vec<JobId>;
    /// Mark a leased job successfully done (drops it entirely).
    fn ack(&self, job: JobId);
    /// Report a leased job failed: requeue it, or dead-letter it once `max_attempts` is reached.
    fn nack(&self, job: JobId) -> Nacked;
    /// Number of jobs waiting to be leased.
    fn pending(&self) -> usize;
    /// Jobs that exhausted their retries.
    fn dead_letters(&self) -> Vec<JobId>;
}

#[derive(Default)]
struct Inner {
    queued: VecDeque<JobId>,
    inflight: HashSet<JobId>,
    attempts: HashMap<JobId, u32>,
    dead: Vec<JobId>,
}

/// In-memory [`Queue`] — the Phase-1 backend and the reference for the Postgres implementation.
pub struct MemQueue {
    inner: Mutex<Inner>,
    max_attempts: u32,
}

impl MemQueue {
    /// A queue that dead-letters a job after `max_attempts` failed tries (must be ≥ 1).
    pub fn new(max_attempts: u32) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            max_attempts: max_attempts.max(1),
        }
    }
}

impl Default for MemQueue {
    /// Five attempts before dead-lettering.
    fn default() -> Self {
        Self::new(5)
    }
}

impl Queue for MemQueue {
    fn enqueue(&self, job: JobId) {
        let mut g = self.inner.lock().unwrap();
        g.attempts.entry(job).or_insert(0);
        g.queued.push_back(job);
    }

    fn lease(&self, max: usize) -> Vec<JobId> {
        let mut g = self.inner.lock().unwrap();
        let mut leased = Vec::new();
        for _ in 0..max {
            let Some(job) = g.queued.pop_front() else {
                break;
            };
            *g.attempts.entry(job).or_insert(0) += 1;
            g.inflight.insert(job);
            leased.push(job);
        }
        leased
    }

    fn ack(&self, job: JobId) {
        let mut g = self.inner.lock().unwrap();
        g.inflight.remove(&job);
        g.attempts.remove(&job);
    }

    fn nack(&self, job: JobId) -> Nacked {
        let mut g = self.inner.lock().unwrap();
        g.inflight.remove(&job);
        let attempt = g.attempts.get(&job).copied().unwrap_or(0);
        if attempt >= self.max_attempts {
            g.attempts.remove(&job);
            g.dead.push(job);
            Nacked::DeadLettered
        } else {
            g.queued.push_back(job);
            Nacked::Requeued { attempt }
        }
    }

    fn pending(&self) -> usize {
        self.inner.lock().unwrap().queued.len()
    }

    fn dead_letters(&self) -> Vec<JobId> {
        self.inner.lock().unwrap().dead.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_lease_ack_flow() {
        let q = MemQueue::default();
        q.enqueue(1);
        q.enqueue(2);
        q.enqueue(3);
        assert_eq!(q.pending(), 3);

        let leased = q.lease(2);
        assert_eq!(leased, vec![1, 2]); // FIFO
        assert_eq!(q.pending(), 1); // leased jobs are no longer pending

        q.ack(1);
        q.ack(2);
        assert_eq!(q.pending(), 1); // job 3 still waiting
    }

    #[test]
    fn nack_requeues_until_dead_letter() {
        let q = MemQueue::new(2); // dead-letter after 2 attempts
        q.enqueue(7);

        // attempt 1
        assert_eq!(q.lease(1), vec![7]);
        assert_eq!(q.nack(7), Nacked::Requeued { attempt: 1 });
        assert_eq!(q.pending(), 1); // back in the queue

        // attempt 2 — exhausts retries
        assert_eq!(q.lease(1), vec![7]);
        assert_eq!(q.nack(7), Nacked::DeadLettered);
        assert_eq!(q.pending(), 0);
        assert_eq!(q.dead_letters(), vec![7]);
    }

    #[test]
    fn lease_on_empty_queue_returns_nothing() {
        let q = MemQueue::default();
        assert!(q.lease(10).is_empty());
    }
}
