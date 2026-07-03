//! The Kerf worker: the loop that turns queued jobs into stored verdicts.
//!
//! [`process_one`] is the pure step — lease a job, load its blobs, run `kerf-engine`, store the
//! immutable result, and `ack`/`nack` the queue. [`run`] drives it asynchronously off the reactor
//! (the ~seconds-long CPU verify runs in `spawn_blocking`) until a shutdown signal. Everything is
//! expressed against the `Store` / `Queue` traits, so the same worker runs over the in-memory backends
//! today and Postgres/object-store in Phase 2.

use std::sync::Arc;
use std::time::Duration;

use kerf_queue::{JobId, Queue};
use kerf_store::{JobSpec, ResultId, Store};

/// The result of attempting one job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The queue was empty.
    Idle,
    /// A job was verified/diffed and its result stored.
    Completed { job: JobId, result: ResultId },
    /// A job could not be processed (missing input, or already completed) — nacked or dropped.
    Failed { job: JobId },
}

/// Lease at most one job and process it to completion. Synchronous and self-contained: this is the unit
/// of work, testable without a runtime.
pub fn process_one(store: &dyn Store, queue: &dyn Queue) -> Outcome {
    let Some(job_id) = queue.lease(1).into_iter().next() else {
        return Outcome::Idle;
    };
    let Some(job) = store.get_job(job_id) else {
        queue.nack(job_id);
        return Outcome::Failed { job: job_id };
    };
    let _ = store.set_running(job_id);

    let envelope = match &job.spec {
        JobSpec::Verify { input } => match store.get_blob(input) {
            Some(bytes) => kerf_engine::verify(&String::from_utf8_lossy(&bytes), job.resolution_um),
            None => {
                queue.nack(job_id);
                return Outcome::Failed { job: job_id };
            }
        },
        JobSpec::Diff { a, b } => match (store.get_blob(a), store.get_blob(b)) {
            (Some(x), Some(y)) => kerf_engine::diff(
                &String::from_utf8_lossy(&x),
                &String::from_utf8_lossy(&y),
                job.resolution_um,
            ),
            _ => {
                queue.nack(job_id);
                return Outcome::Failed { job: job_id };
            }
        },
    };

    match store.complete_job(job_id, envelope) {
        Ok(result) => {
            queue.ack(job_id);
            Outcome::Completed {
                job: job_id,
                result,
            }
        }
        // Already completed (e.g. a duplicate delivery): drop it, don't retry.
        Err(_) => {
            queue.ack(job_id);
            Outcome::Failed { job: job_id }
        }
    }
}

/// Run the worker loop until `shutdown` flips to `true`. The CPU-bound engine call runs on the blocking
/// pool so it never stalls the async reactor; an empty queue backs off briefly.
pub async fn run(
    store: Arc<dyn Store>,
    queue: Arc<dyn Queue>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        let (s, q) = (store.clone(), queue.clone());
        let outcome = tokio::task::spawn_blocking(move || process_one(s.as_ref(), q.as_ref()))
            .await
            .unwrap_or(Outcome::Idle);
        if outcome == Outcome::Idle {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                _ = shutdown.changed() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_queue::MemQueue;
    use kerf_store::MemStore;

    const GCODE: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:External perimeter\n;WIDTH:0.45\nG0 X0 Y0\nG1 X10 Y0 E.4\nG1 X10 Y10 E.4";

    #[test]
    fn processes_a_verify_job_end_to_end() {
        let store = MemStore::new();
        let queue = MemQueue::default();
        let blob = store.put_blob(GCODE.as_bytes());
        let job = store.create_job("acme", JobSpec::Verify { input: blob }, 200);
        queue.enqueue(job);

        let outcome = process_one(&store, &queue);
        let Outcome::Completed { job: j, result } = outcome else {
            panic!("expected Completed, got {outcome:?}");
        };
        assert_eq!(j, job);
        let stored = store.get_result(result).unwrap();
        assert_eq!(stored.envelope.summary.ok, Some(true));
        assert_eq!(queue.pending(), 0);
        assert!(queue.dead_letters().is_empty());
    }

    #[test]
    fn idle_when_queue_empty() {
        let store = MemStore::new();
        let queue = MemQueue::default();
        assert_eq!(process_one(&store, &queue), Outcome::Idle);
    }

    #[test]
    fn missing_blob_fails_and_nacks() {
        let store = MemStore::new();
        let queue = MemQueue::new(2);
        // A job whose input blob was never stored.
        let job = store.create_job(
            "acme",
            JobSpec::Verify {
                input: "deadbeef".into(),
            },
            200,
        );
        queue.enqueue(job);
        assert_eq!(process_one(&store, &queue), Outcome::Failed { job });
        assert_eq!(queue.pending(), 1); // requeued (attempt 1 of 2)
    }
}
