//! `kerf-serve` — a single binary running the API and a background worker over the in-memory backends.
//! The demoable Phase-1 spine: `POST /v1/verify`, the worker processes it, `GET /v1/results/{id}`.
//!
//! Config via env: `KERF_ADDR` (default `0.0.0.0:8080`), `KERF_API_KEY` (default `dev-key`),
//! `KERF_TENANT` (default `default`).

use std::collections::HashMap;
use std::sync::Arc;

use kerf_api::{build_router, AppState, Metrics, Role};
use kerf_queue::{MemQueue, Queue};
use kerf_store::{MemStore, Store};

#[tokio::main]
async fn main() {
    let store: Arc<dyn Store> = select_store();
    let queue: Arc<dyn Queue> = Arc::new(MemQueue::default());

    let key = std::env::var("KERF_API_KEY").unwrap_or_else(|_| "dev-key".to_string());
    let tenant = std::env::var("KERF_TENANT").unwrap_or_else(|_| "default".to_string());
    let mut key_map = HashMap::from([(key.clone(), (tenant.clone(), Role::Writer))]);
    // Optional read-only key (RBAC): KERF_READER_KEY may read but not submit.
    if let Ok(ro) = std::env::var("KERF_READER_KEY") {
        if !ro.is_empty() {
            key_map.insert(ro, (tenant.clone(), Role::Reader));
        }
    }
    let keys = Arc::new(key_map);

    let state = AppState {
        store: store.clone(),
        queue: queue.clone(),
        keys,
        metrics: Arc::new(Metrics::default()),
    };

    // Background worker loop, stopped on shutdown.
    let (tx, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(kerf_worker::run(store, queue, rx));

    let addr = std::env::var("KERF_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind KERF_ADDR");
    eprintln!("kerf-serve listening on {addr} (X-API-Key: {key})");

    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = tx.send(true); // stop the worker too
        })
        .await
        .expect("server error");

    let _ = worker.await;
}

/// Pick the backend: durable Postgres when `DATABASE_URL` is set (and built with `--features postgres`),
/// otherwise the in-memory store.
fn select_store() -> Arc<dyn Store> {
    match std::env::var("DATABASE_URL") {
        Ok(url) if !url.is_empty() => {
            #[cfg(feature = "postgres")]
            {
                eprintln!("kerf: using Postgres store");
                let store = kerf_store::PgStore::connect_retry(&url, 30)
                    .expect("connect to Postgres (DATABASE_URL)");
                Arc::new(store)
            }
            #[cfg(not(feature = "postgres"))]
            {
                eprintln!(
                    "kerf: DATABASE_URL is set but this build lacks the `postgres` feature; \
                     using the in-memory store"
                );
                Arc::new(MemStore::new())
            }
        }
        _ => {
            eprintln!("kerf: using in-memory store (set DATABASE_URL for Postgres)");
            Arc::new(MemStore::new())
        }
    }
}
