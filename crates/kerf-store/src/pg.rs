//! Postgres-backed [`Store`] (feature `postgres`) — the durable backend behind the same trait the
//! in-memory [`MemStore`](crate::MemStore) implements, so nothing else in the platform changes.
//!
//! The synchronous `postgres` client runs its own internal runtime, which cannot be driven from inside
//! the server's Tokio runtime ("cannot start a runtime from within a runtime"). So the client lives on a
//! dedicated OS thread and every [`Store`] call is dispatched to it over a channel and awaited on a
//! reply channel. This keeps the `Store` trait synchronous and works uniformly from async handlers and
//! from the worker's blocking threads.
//!
//! Results are immutable in app logic (re-completing errors) AND at the database level (a trigger
//! rejects UPDATE/DELETE on `results`), and are linked into a per-tenant hash chain — the auditability
//! spine.

use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;

use postgres::{Client, NoTls, Row};

use crate::{
    blob_id, Alert, Baseline, BlobId, Job, JobId, JobSpec, JobStatus, ResultId, Store, StoreError,
    StoredResult,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS blobs (
    id     TEXT PRIMARY KEY,
    bytes  BYTEA NOT NULL
);
CREATE TABLE IF NOT EXISTS jobs (
    id             BIGSERIAL PRIMARY KEY,
    tenant         TEXT   NOT NULL,
    spec           JSONB  NOT NULL,
    resolution_um  BIGINT NOT NULL,
    status         TEXT   NOT NULL,
    result_id      BIGINT
);
CREATE TABLE IF NOT EXISTS results (
    id             BIGSERIAL PRIMARY KEY,
    job_id         BIGINT NOT NULL,
    tenant         TEXT   NOT NULL,
    chain_seq      BIGINT NOT NULL,
    prev_digest    TEXT,
    result_digest  TEXT   NOT NULL,
    envelope       JSONB  NOT NULL
);
CREATE TABLE IF NOT EXISTS baselines (
    tenant         TEXT   NOT NULL,
    project        TEXT   NOT NULL,
    blob           TEXT   NOT NULL,
    resolution_um  BIGINT NOT NULL,
    PRIMARY KEY (tenant, project)
);
CREATE TABLE IF NOT EXISTS alerts (
    id         BIGSERIAL PRIMARY KEY,
    tenant     TEXT   NOT NULL,
    project    TEXT   NOT NULL,
    result_id  BIGINT NOT NULL,
    iou        DOUBLE PRECISION,
    message    TEXT   NOT NULL
);
-- Enforce result immutability at the database level (auditability).
CREATE OR REPLACE FUNCTION kerf_results_immutable() RETURNS trigger AS $$
BEGIN RAISE EXCEPTION 'kerf: results are immutable'; END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS kerf_results_no_mutate ON results;
CREATE TRIGGER kerf_results_no_mutate BEFORE UPDATE OR DELETE ON results
    FOR EACH ROW EXECUTE FUNCTION kerf_results_immutable();
"#;

type Task = Box<dyn FnOnce(&mut Client) + Send>;

/// A durable Postgres-backed store. Holds a handle to a dedicated DB thread.
pub struct PgStore {
    tx: Sender<Task>,
}

impl PgStore {
    /// Connect (retrying while the database comes up) and apply the schema on a dedicated thread.
    pub fn connect_retry(url: &str, attempts: u32) -> Result<Self, String> {
        let (tx, rx) = channel::<Task>();
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        let url = url.to_string();
        thread::spawn(move || {
            let mut client = match connect_blocking(&url, attempts) {
                Ok(c) => c,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            while let Ok(task) = rx.recv() {
                task(&mut client);
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("postgres worker thread exited before ready".to_string()),
        }
    }

    /// Dispatch a closure to the DB thread and block for its result.
    fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut Client) -> R + Send + 'static) -> R {
        let (rtx, rrx) = channel::<R>();
        self.tx
            .send(Box::new(move |c| {
                let _ = rtx.send(f(c));
            }))
            .expect("db thread alive");
        rrx.recv().expect("db thread response")
    }
}

fn connect_blocking(url: &str, attempts: u32) -> Result<Client, String> {
    let mut last = String::from("no attempts");
    for i in 0..attempts.max(1) {
        match Client::connect(url, NoTls) {
            Ok(mut c) => {
                c.batch_execute(SCHEMA).map_err(|e| e.to_string())?;
                return Ok(c);
            }
            Err(e) => {
                eprintln!("kerf: waiting for postgres (attempt {i}): {e}");
                last = e.to_string();
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
    Err(last)
}

fn status_from(s: &str) -> JobStatus {
    match s {
        "running" => JobStatus::Running,
        "done" => JobStatus::Done,
        "failed" => JobStatus::Failed,
        _ => JobStatus::Queued,
    }
}

fn row_to_job(row: &Row) -> Job {
    let id: i64 = row.get(0);
    let spec: serde_json::Value = row.get(2);
    let resolution_um: i64 = row.get(3);
    let status: String = row.get(4);
    let result_id: Option<i64> = row.get(5);
    Job {
        id: id as u64,
        tenant: row.get(1),
        spec: serde_json::from_value(spec).expect("valid JobSpec"),
        resolution_um,
        status: status_from(&status),
        result_id: result_id.map(|v| v as u64),
    }
}

fn row_to_result(row: &Row) -> StoredResult {
    let id: i64 = row.get(0);
    let job_id: i64 = row.get(1);
    let chain_seq: i64 = row.get(3);
    let envelope: serde_json::Value = row.get(6);
    StoredResult {
        id: id as u64,
        job_id: job_id as u64,
        tenant: row.get(2),
        chain_seq: chain_seq as u64,
        prev_digest: row.get(4),
        result_digest: row.get(5),
        envelope: serde_json::from_value(envelope).expect("valid VerdictEnvelope"),
    }
}

impl Store for PgStore {
    fn put_blob(&self, bytes: &[u8]) -> BlobId {
        let id = blob_id(bytes);
        let id2 = id.clone();
        let data = bytes.to_vec();
        self.call(move |c| {
            c.execute(
                "INSERT INTO blobs (id, bytes) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
                &[&id2, &data],
            )
            .expect("put_blob");
        });
        id
    }

    fn get_blob(&self, id: &BlobId) -> Option<Vec<u8>> {
        let id = id.clone();
        self.call(move |c| {
            c.query_opt("SELECT bytes FROM blobs WHERE id = $1", &[&id])
                .expect("get_blob")
                .map(|r| r.get::<_, Vec<u8>>(0))
        })
    }

    fn create_job(&self, tenant: &str, spec: JobSpec, resolution_um: i64) -> JobId {
        let tenant = tenant.to_string();
        let spec_json = serde_json::to_value(&spec).expect("spec json");
        self.call(move |c| {
            c.query_one(
                "INSERT INTO jobs (tenant, spec, resolution_um, status) \
                 VALUES ($1, $2, $3, 'queued') RETURNING id",
                &[&tenant, &spec_json, &resolution_um],
            )
            .expect("create_job")
            .get::<_, i64>(0) as u64
        })
    }

    fn get_job(&self, id: JobId) -> Option<Job> {
        let key = id as i64;
        self.call(move |c| {
            c.query_opt(
                "SELECT id, tenant, spec, resolution_um, status, result_id FROM jobs WHERE id = $1",
                &[&key],
            )
            .expect("get_job")
            .as_ref()
            .map(row_to_job)
        })
    }

    fn set_running(&self, id: JobId) -> Result<(), StoreError> {
        let key = id as i64;
        self.call(move |c| {
            let row = c
                .query_opt("SELECT status FROM jobs WHERE id = $1", &[&key])
                .expect("set_running select");
            match row {
                None => Err(StoreError::JobNotFound(id)),
                Some(r) => {
                    let s: String = r.get(0);
                    if s == "done" {
                        return Err(StoreError::AlreadyCompleted(id));
                    }
                    c.execute("UPDATE jobs SET status = 'running' WHERE id = $1", &[&key])
                        .expect("set_running update");
                    Ok(())
                }
            }
        })
    }

    fn complete_job(
        &self,
        id: JobId,
        envelope: kerf_engine::VerdictEnvelope,
    ) -> Result<ResultId, StoreError> {
        let key = id as i64;
        self.call(move |c| {
            let mut tx = c.transaction().expect("begin tx");
            let row = tx
                .query_opt("SELECT tenant, status FROM jobs WHERE id = $1 FOR UPDATE", &[&key])
                .expect("lock job");
            let (tenant, status): (String, String) = match row {
                None => return Err(StoreError::JobNotFound(id)),
                Some(r) => (r.get(0), r.get(1)),
            };
            if status == "done" {
                return Err(StoreError::AlreadyCompleted(id));
            }
            let result_digest = envelope.result_digest();
            let tail = tx
                .query_opt(
                    "SELECT chain_seq, result_digest FROM results WHERE tenant = $1 \
                     ORDER BY chain_seq DESC LIMIT 1",
                    &[&tenant],
                )
                .expect("chain tail");
            let (chain_seq, prev_digest): (i64, Option<String>) = match tail {
                Some(r) => (r.get::<_, i64>(0) + 1, Some(r.get::<_, String>(1))),
                None => (0, None),
            };
            let envelope_json = serde_json::to_value(&envelope).expect("envelope json");
            let rid: i64 = tx
                .query_one(
                    "INSERT INTO results (job_id, tenant, chain_seq, prev_digest, result_digest, envelope) \
                     VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
                    &[&key, &tenant, &chain_seq, &prev_digest, &result_digest, &envelope_json],
                )
                .expect("insert result")
                .get(0);
            tx.execute(
                "UPDATE jobs SET status = 'done', result_id = $1 WHERE id = $2",
                &[&rid, &key],
            )
            .expect("mark done");
            tx.commit().expect("commit");
            Ok(rid as u64)
        })
    }

    fn get_result(&self, id: ResultId) -> Option<StoredResult> {
        let key = id as i64;
        self.call(move |c| {
            c.query_opt(
                "SELECT id, job_id, tenant, chain_seq, prev_digest, result_digest, envelope \
                 FROM results WHERE id = $1",
                &[&key],
            )
            .expect("get_result")
            .as_ref()
            .map(row_to_result)
        })
    }

    fn list_jobs(&self, tenant: &str) -> Vec<Job> {
        let tenant = tenant.to_string();
        self.call(move |c| {
            c.query(
                "SELECT id, tenant, spec, resolution_um, status, result_id FROM jobs \
                 WHERE tenant = $1 ORDER BY id",
                &[&tenant],
            )
            .expect("list_jobs")
            .iter()
            .map(row_to_job)
            .collect()
        })
    }

    fn set_baseline(&self, tenant: &str, project: &str, blob: BlobId, resolution_um: i64) {
        let tenant = tenant.to_string();
        let project = project.to_string();
        self.call(move |c| {
            c.execute(
                "INSERT INTO baselines (tenant, project, blob, resolution_um) VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (tenant, project) DO UPDATE SET blob = EXCLUDED.blob, resolution_um = EXCLUDED.resolution_um",
                &[&tenant, &project, &blob, &resolution_um],
            )
            .expect("set_baseline");
        });
    }

    fn get_baseline(&self, tenant: &str, project: &str) -> Option<Baseline> {
        let tenant = tenant.to_string();
        let project = project.to_string();
        self.call(move |c| {
            c.query_opt(
                "SELECT tenant, project, blob, resolution_um FROM baselines WHERE tenant = $1 AND project = $2",
                &[&tenant, &project],
            )
            .expect("get_baseline")
            .map(|row| Baseline {
                tenant: row.get(0),
                project: row.get(1),
                blob: row.get(2),
                resolution_um: row.get(3),
            })
        })
    }

    fn record_alert(
        &self,
        tenant: &str,
        project: &str,
        result_id: ResultId,
        iou: Option<f64>,
        message: &str,
    ) -> u64 {
        let tenant = tenant.to_string();
        let project = project.to_string();
        let message = message.to_string();
        let rid = result_id as i64;
        self.call(move |c| {
            c.query_one(
                "INSERT INTO alerts (tenant, project, result_id, iou, message) \
                 VALUES ($1, $2, $3, $4, $5) RETURNING id",
                &[&tenant, &project, &rid, &iou, &message],
            )
            .expect("record_alert")
            .get::<_, i64>(0) as u64
        })
    }

    fn list_alerts(&self, tenant: &str) -> Vec<Alert> {
        let tenant = tenant.to_string();
        self.call(move |c| {
            c.query(
                "SELECT id, tenant, project, result_id, iou, message FROM alerts WHERE tenant = $1 ORDER BY id",
                &[&tenant],
            )
            .expect("list_alerts")
            .iter()
            .map(|r| Alert {
                id: r.get::<_, i64>(0) as u64,
                tenant: r.get(1),
                project: r.get(2),
                result_id: r.get::<_, i64>(3) as u64,
                iou: r.get(4),
                message: r.get(5),
            })
            .collect()
        })
    }
}
