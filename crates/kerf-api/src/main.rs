//! `kerf-serve` — a single binary running the API and a background worker over the in-memory backends.
//! The demoable Phase-1 spine: `POST /v1/verify`, the worker processes it, `GET /v1/results/{id}`.
//!
//! Config via env: `KERF_ADDR` (default `0.0.0.0:8080`), `KERF_API_KEY` (default `dev-key`),
//! `KERF_TENANT` (default `default`).

use std::collections::HashMap;
use std::sync::Arc;

use kerf_api::{build_router, AppState};
use kerf_queue::{MemQueue, Queue};
use kerf_store::{MemStore, Store};

#[tokio::main]
async fn main() {
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let queue: Arc<dyn Queue> = Arc::new(MemQueue::default());

    let key = std::env::var("KERF_API_KEY").unwrap_or_else(|_| "dev-key".to_string());
    let tenant = std::env::var("KERF_TENANT").unwrap_or_else(|_| "default".to_string());
    let keys = Arc::new(HashMap::from([(key.clone(), tenant)]));

    let state = AppState {
        store: store.clone(),
        queue: queue.clone(),
        keys,
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
