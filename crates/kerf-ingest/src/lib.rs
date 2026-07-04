//! Watch-folder ingest: each new `*.gcode` file is content-addressed into the store and enqueued as a
//! verify job. Idempotent (a file already seen is not resubmitted), so scans can run on a timer.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use kerf_queue::{JobId, Queue};
use kerf_store::{JobSpec, Store};

/// Scans a directory for new G-code files and submits them.
pub struct Ingester {
    dir: PathBuf,
    tenant: String,
    resolution_um: i64,
    seen: HashSet<PathBuf>,
}

impl Ingester {
    pub fn new(dir: impl Into<PathBuf>, tenant: impl Into<String>, resolution_um: i64) -> Self {
        Self {
            dir: dir.into(),
            tenant: tenant.into(),
            resolution_um,
            seen: HashSet::new(),
        }
    }

    /// Submit every `*.gcode` file not seen before; returns the `(path, job_id)` submitted this scan.
    pub fn scan(&mut self, store: &dyn Store, queue: &dyn Queue) -> Vec<(PathBuf, JobId)> {
        let mut submitted = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return submitted;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_gcode(p))
            .collect();
        paths.sort(); // deterministic submission order
        for path in paths {
            if self.seen.contains(&path) {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            let blob = store.put_blob(&bytes);
            let job = store.create_job(
                &self.tenant,
                JobSpec::Verify { input: blob },
                self.resolution_um,
            );
            queue.enqueue(job);
            self.seen.insert(path.clone());
            submitted.push((path, job));
        }
        submitted
    }
}

fn is_gcode(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gcode"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_queue::MemQueue;
    use kerf_store::MemStore;

    const GCODE: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\nG1 X10 Y0 E.4";

    #[test]
    fn scans_new_gcode_once_and_ignores_others() {
        let dir = std::env::temp_dir().join(format!("kerf-ingest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.gcode"), GCODE).unwrap();
        fs::write(dir.join("notes.txt"), "not gcode").unwrap();

        let store = MemStore::new();
        let queue = MemQueue::default();
        let mut ing = Ingester::new(&dir, "acme", 200);

        assert_eq!(ing.scan(&store, &queue).len(), 1);
        assert_eq!(queue.pending(), 1);
        assert_eq!(ing.scan(&store, &queue).len(), 0);
        fs::write(dir.join("b.gcode"), GCODE).unwrap();
        let third = ing.scan(&store, &queue);
        assert_eq!(third.len(), 1);
        assert_eq!(store.list_jobs("acme").len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let store = MemStore::new();
        let queue = MemQueue::default();
        let mut ing = Ingester::new("/no/such/kerf/dir", "acme", 200);
        assert!(ing.scan(&store, &queue).is_empty());
    }
}
