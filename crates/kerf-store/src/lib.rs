//! Persistence for the Kerf platform, behind a single [`Store`] trait so the API and workers depend on
//! one interface. This crate ships the in-memory implementation ([`MemStore`]) that makes the whole
//! service runnable and testable today; the Postgres + object-store implementation (Phase 2) slots in
//! behind the same trait without touching callers.
//!
//! Design invariants that hold regardless of backend:
//!  - **Blobs are content-addressed** (SHA-256), so identical inputs are stored once.
//!  - **Results are immutable** — [`Store::complete_job`] inserts a result and can never overwrite one;
//!    completing an already-done job is an error.
//!  - **Results form a per-tenant append-only chain** — each carries the previous result's digest, so
//!    tampering with history is detectable (the auditability spine).

use std::collections::HashMap;
use std::sync::Mutex;

use kerf_engine::VerdictEnvelope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Lowercase-hex SHA-256 of a blob's bytes.
pub type BlobId = String;
pub type JobId = u64;
pub type ResultId = u64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("job {0} not found")]
    JobNotFound(JobId),
    #[error("job {0} is already completed (results are immutable)")]
    AlreadyCompleted(JobId),
}

/// What a job asks the engine to compute.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum JobSpec {
    Verify { input: BlobId },
    Diff { a: BlobId, b: BlobId },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub tenant: String,
    pub spec: JobSpec,
    pub resolution_um: i64,
    pub status: JobStatus,
    pub result_id: Option<ResultId>,
}

/// An immutable verdict record, linked into its tenant's audit chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredResult {
    pub id: ResultId,
    pub job_id: JobId,
    pub tenant: String,
    /// Position in the tenant's append-only chain (0-based).
    pub chain_seq: u64,
    /// `result_digest` of the previous result in this tenant's chain (`None` for the first).
    pub prev_digest: Option<String>,
    /// This verdict's reproducible digest (see [`VerdictEnvelope::result_digest`]).
    pub result_digest: String,
    pub envelope: VerdictEnvelope,
}

/// Content-address a blob.
pub fn blob_id(bytes: &[u8]) -> BlobId {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The persistence interface. Postgres/object-store (Phase 2) implements exactly this.
pub trait Store: Send + Sync {
    /// Store a blob content-addressed; returns its id (deduplicated).
    fn put_blob(&self, bytes: &[u8]) -> BlobId;
    fn get_blob(&self, id: &BlobId) -> Option<Vec<u8>>;
    fn create_job(&self, tenant: &str, spec: JobSpec, resolution_um: i64) -> JobId;
    fn get_job(&self, id: JobId) -> Option<Job>;
    fn set_running(&self, id: JobId) -> Result<(), StoreError>;
    /// Insert the immutable result of a job and mark it done. Errors if already completed.
    fn complete_job(&self, id: JobId, envelope: VerdictEnvelope) -> Result<ResultId, StoreError>;
    fn get_result(&self, id: ResultId) -> Option<StoredResult>;
    fn list_jobs(&self, tenant: &str) -> Vec<Job>;
}

#[derive(Default)]
struct Inner {
    blobs: HashMap<BlobId, Vec<u8>>,
    jobs: HashMap<JobId, Job>,
    results: HashMap<ResultId, StoredResult>,
    next_job: JobId,
    next_result: ResultId,
    /// Per-tenant chain tail: (last chain_seq, last result_digest).
    chain: HashMap<String, (u64, String)>,
}

/// In-memory [`Store`] — the Phase-1 backend; also the reference for the Postgres implementation.
#[derive(Default)]
pub struct MemStore {
    inner: Mutex<Inner>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemStore {
    fn put_blob(&self, bytes: &[u8]) -> BlobId {
        let id = blob_id(bytes);
        let mut g = self.inner.lock().unwrap();
        g.blobs.entry(id.clone()).or_insert_with(|| bytes.to_vec());
        id
    }

    fn get_blob(&self, id: &BlobId) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().blobs.get(id).cloned()
    }

    fn create_job(&self, tenant: &str, spec: JobSpec, resolution_um: i64) -> JobId {
        let mut g = self.inner.lock().unwrap();
        let id = g.next_job;
        g.next_job += 1;
        g.jobs.insert(
            id,
            Job {
                id,
                tenant: tenant.to_string(),
                spec,
                resolution_um,
                status: JobStatus::Queued,
                result_id: None,
            },
        );
        id
    }

    fn get_job(&self, id: JobId) -> Option<Job> {
        self.inner.lock().unwrap().jobs.get(&id).cloned()
    }

    fn set_running(&self, id: JobId) -> Result<(), StoreError> {
        let mut g = self.inner.lock().unwrap();
        let job = g.jobs.get_mut(&id).ok_or(StoreError::JobNotFound(id))?;
        if job.status == JobStatus::Done {
            return Err(StoreError::AlreadyCompleted(id));
        }
        job.status = JobStatus::Running;
        Ok(())
    }

    fn complete_job(&self, id: JobId, envelope: VerdictEnvelope) -> Result<ResultId, StoreError> {
        let mut g = self.inner.lock().unwrap();
        let (tenant, status) = match g.jobs.get(&id) {
            None => return Err(StoreError::JobNotFound(id)),
            Some(j) => (j.tenant.clone(), j.status),
        };
        if status == JobStatus::Done {
            return Err(StoreError::AlreadyCompleted(id));
        }
        let result_digest = envelope.result_digest();
        let (chain_seq, prev_digest) = match g.chain.get(&tenant) {
            Some((seq, last)) => (seq + 1, Some(last.clone())),
            None => (0, None),
        };
        let rid = g.next_result;
        g.next_result += 1;
        g.results.insert(
            rid,
            StoredResult {
                id: rid,
                job_id: id,
                tenant: tenant.clone(),
                chain_seq,
                prev_digest,
                result_digest: result_digest.clone(),
                envelope,
            },
        );
        g.chain.insert(tenant, (chain_seq, result_digest));
        if let Some(job) = g.jobs.get_mut(&id) {
            job.status = JobStatus::Done;
            job.result_id = Some(rid);
        }
        Ok(rid)
    }

    fn get_result(&self, id: ResultId) -> Option<StoredResult> {
        self.inner.lock().unwrap().results.get(&id).cloned()
    }

    fn list_jobs(&self, tenant: &str) -> Vec<Job> {
        let mut v: Vec<Job> = self
            .inner
            .lock()
            .unwrap()
            .jobs
            .values()
            .filter(|j| j.tenant == tenant)
            .cloned()
            .collect();
        v.sort_by_key(|j| j.id);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GCODE: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:External perimeter\n;WIDTH:0.45\nG0 X0 Y0\nG1 X10 Y0 E.4\nG1 X10 Y10 E.4";

    #[test]
    fn blobs_are_content_addressed_and_deduped() {
        let s = MemStore::new();
        let id1 = s.put_blob(GCODE.as_bytes());
        let id2 = s.put_blob(GCODE.as_bytes());
        assert_eq!(id1, id2); // same content → same id
        assert_eq!(id1.len(), 64);
        assert_eq!(s.get_blob(&id1).as_deref(), Some(GCODE.as_bytes()));
        assert_ne!(id1, s.put_blob(b"different"));
        assert_eq!(s.get_blob(&"deadbeef".to_string()), None);
    }

    #[test]
    fn job_lifecycle_and_immutable_result() {
        let s = MemStore::new();
        let blob = s.put_blob(GCODE.as_bytes());
        let job = s.create_job("acme", JobSpec::Verify { input: blob }, 200);
        assert_eq!(s.get_job(job).unwrap().status, JobStatus::Queued);
        s.set_running(job).unwrap();
        assert_eq!(s.get_job(job).unwrap().status, JobStatus::Running);

        let env = kerf_engine::verify(GCODE, 200);
        let digest = env.result_digest();
        let rid = s.complete_job(job, env).unwrap();

        let stored = s.get_result(rid).unwrap();
        assert_eq!(stored.result_digest, digest);
        assert_eq!(stored.chain_seq, 0);
        assert_eq!(stored.prev_digest, None);
        let done = s.get_job(job).unwrap();
        assert_eq!(done.status, JobStatus::Done);
        assert_eq!(done.result_id, Some(rid));

        // Immutability: a completed job cannot be completed again.
        let err = s
            .complete_job(job, kerf_engine::verify(GCODE, 200))
            .unwrap_err();
        assert_eq!(err, StoreError::AlreadyCompleted(job));
    }

    #[test]
    fn results_form_a_per_tenant_audit_chain() {
        let s = MemStore::new();
        let blob = s.put_blob(GCODE.as_bytes());
        let j1 = s.create_job(
            "acme",
            JobSpec::Verify {
                input: blob.clone(),
            },
            200,
        );
        let j2 = s.create_job("acme", JobSpec::Verify { input: blob }, 100);
        let r1 = s.complete_job(j1, kerf_engine::verify(GCODE, 200)).unwrap();
        let r2 = s.complete_job(j2, kerf_engine::verify(GCODE, 100)).unwrap();

        let s1 = s.get_result(r1).unwrap();
        let s2 = s.get_result(r2).unwrap();
        assert_eq!(s1.chain_seq, 0);
        assert_eq!(s2.chain_seq, 1);
        // The second result links to the first — a tamper-evident chain.
        assert_eq!(s2.prev_digest.as_deref(), Some(s1.result_digest.as_str()));
    }

    #[test]
    fn tenants_are_isolated_in_listing() {
        let s = MemStore::new();
        let blob = s.put_blob(GCODE.as_bytes());
        s.create_job(
            "acme",
            JobSpec::Verify {
                input: blob.clone(),
            },
            200,
        );
        s.create_job("globex", JobSpec::Verify { input: blob }, 200);
        assert_eq!(s.list_jobs("acme").len(), 1);
        assert_eq!(s.list_jobs("globex").len(), 1);
        assert_eq!(s.list_jobs("nobody").len(), 0);
    }
}
