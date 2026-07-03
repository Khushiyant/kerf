//! The Kerf platform HTTP API: submit verify/diff jobs, poll them, and read the immutable result — plus
//! a minimal browser dashboard. Stateless; wired to a [`Store`] and a [`Queue`] (in-memory today,
//! Postgres in Phase 2). Every request is authenticated by an `X-API-Key` header mapped to a tenant,
//! and every read is tenant-scoped so one tenant can never see another's jobs or results.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use kerf_queue::Queue;
use kerf_store::{Job, JobSpec, Store, StoredResult};

const DEFAULT_RESOLUTION_UM: i64 = 200;

/// Shared service state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub queue: Arc<dyn Queue>,
    /// API key → tenant.
    pub keys: Arc<HashMap<String, String>>,
}

/// Build the router. `GET /` serves the dashboard; `/healthz` + `/readyz` are unauthenticated probes;
/// everything under `/v1` requires a valid `X-API-Key`.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }))
        .route("/v1/verify", post(post_verify))
        .route("/v1/diff", post(post_diff))
        .route("/v1/jobs", get(list_jobs))
        .route("/v1/jobs/{id}", get(get_job))
        .route("/v1/results/{id}", get(get_result))
        .with_state(state)
}

/// Resolve the tenant for a request, or reject with 401.
fn authorize(headers: &HeaderMap, st: &AppState) -> Result<String, StatusCode> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    st.keys.get(key).cloned().ok_or(StatusCode::UNAUTHORIZED)
}

#[derive(Deserialize)]
struct VerifyReq {
    gcode: String,
    #[serde(default)]
    resolution_um: Option<i64>,
}

#[derive(Deserialize)]
struct DiffReq {
    a: String,
    b: String,
    #[serde(default)]
    resolution_um: Option<i64>,
}

#[derive(Serialize)]
struct JobAccepted {
    job_id: u64,
}

fn resolution(req: Option<i64>) -> Result<i64, StatusCode> {
    let r = req.unwrap_or(DEFAULT_RESOLUTION_UM);
    if r > 0 {
        Ok(r)
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}

async fn post_verify(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<VerifyReq>,
) -> Result<(StatusCode, Json<JobAccepted>), StatusCode> {
    let tenant = authorize(&headers, &st)?;
    let res = resolution(req.resolution_um)?;
    let blob = st.store.put_blob(req.gcode.as_bytes());
    let job = st
        .store
        .create_job(&tenant, JobSpec::Verify { input: blob }, res);
    st.queue.enqueue(job);
    Ok((StatusCode::ACCEPTED, Json(JobAccepted { job_id: job })))
}

async fn post_diff(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DiffReq>,
) -> Result<(StatusCode, Json<JobAccepted>), StatusCode> {
    let tenant = authorize(&headers, &st)?;
    let res = resolution(req.resolution_um)?;
    let a = st.store.put_blob(req.a.as_bytes());
    let b = st.store.put_blob(req.b.as_bytes());
    let job = st.store.create_job(&tenant, JobSpec::Diff { a, b }, res);
    st.queue.enqueue(job);
    Ok((StatusCode::ACCEPTED, Json(JobAccepted { job_id: job })))
}

async fn get_job(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<Job>, StatusCode> {
    let tenant = authorize(&headers, &st)?;
    let job = st
        .store
        .get_job(id)
        .filter(|j| j.tenant == tenant)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(job))
}

async fn get_result(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<StoredResult>, StatusCode> {
    let tenant = authorize(&headers, &st)?;
    let result = st
        .store
        .get_result(id)
        .filter(|r| r.tenant == tenant)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(result))
}

async fn list_jobs(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Job>>, StatusCode> {
    let tenant = authorize(&headers, &st)?;
    Ok(Json(st.store.list_jobs(&tenant)))
}

async fn dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

const DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Kerf</title>
<style>body{font:14px system-ui;margin:2rem;max-width:52rem}textarea{width:100%;height:9rem}
pre{background:#f4f4f4;padding:1rem;overflow:auto}input,button{font:inherit;padding:.3rem}</style></head>
<body><h1>Kerf</h1><p>Verify G-code by the material it deposits.</p>
<p>API key <input id="key" value="dev-key"> resolution µm <input id="res" value="200" size="5"></p>
<textarea id="g" placeholder="paste G-code here"></textarea>
<p><button onclick="run()">Verify</button></p><pre id="out">(result appears here)</pre>
<script>
async function run(){
 const key=document.getElementById('key').value, res=+document.getElementById('res').value;
 const out=document.getElementById('out'); out.textContent='submitting...';
 const h={'content-type':'application/json','x-api-key':key};
 let r=await fetch('/v1/verify',{method:'POST',headers:h,body:JSON.stringify({gcode:document.getElementById('g').value,resolution_um:res})});
 if(!r.ok){out.textContent='error '+r.status;return}
 const {job_id}=await r.json();
 for(let i=0;i<50;i++){await new Promise(s=>setTimeout(s,120));
  const j=await(await fetch('/v1/jobs/'+job_id,{headers:{'x-api-key':key}})).json();
  if(j.status==='done'){const res=await(await fetch('/v1/results/'+j.result_id,{headers:{'x-api-key':key}})).json();
   out.textContent=JSON.stringify(res.envelope.summary,null,2)+'\n\ndigest '+res.result_digest;return}}
 out.textContent='timed out waiting for worker';
}
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use kerf_queue::MemQueue;
    use kerf_store::MemStore;
    use tower::ServiceExt;

    const GCODE: &str = "M83\nG21\n;LAYER_CHANGE\n;Z:0.2\n;TYPE:External perimeter\n;WIDTH:0.45\nG0 X0 Y0\nG1 X10 Y0 E.4\nG1 X10 Y10 E.4";

    fn state() -> (AppState, Arc<dyn Store>, Arc<dyn Queue>) {
        let store: Arc<dyn Store> = Arc::new(MemStore::new());
        let queue: Arc<dyn Queue> = Arc::new(MemQueue::default());
        let keys = Arc::new(HashMap::from([("dev".to_string(), "acme".to_string())]));
        (
            AppState {
                store: store.clone(),
                queue: queue.clone(),
                keys,
            },
            store,
            queue,
        )
    }

    async fn send(app: Router, req: Request<Body>) -> (StatusCode, String) {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn verify_req(key: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method("POST")
            .uri("/v1/verify")
            .header("content-type", "application/json");
        if let Some(k) = key {
            b = b.header("x-api-key", k);
        }
        b.body(Body::from(
            serde_json::to_vec(&serde_json::json!({"gcode": GCODE, "resolution_um": 200})).unwrap(),
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn health_is_unauthenticated() {
        let (st, _, _) = state();
        let (code, body) = send(
            build_router(st),
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn verify_requires_a_valid_key() {
        let (st, _, _) = state();
        let (code, _) = send(build_router(st.clone()), verify_req(None)).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
        let (code, _) = send(build_router(st), verify_req(Some("wrong"))).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn submit_process_and_read_result_end_to_end() {
        let (st, store, queue) = state();
        let app = build_router(st);

        // 1. Submit a verify job over HTTP.
        let (code, body) = send(app.clone(), verify_req(Some("dev"))).await;
        assert_eq!(code, StatusCode::ACCEPTED);
        let job_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["job_id"]
            .as_u64()
            .unwrap();

        // 2. A worker processes it (same shared store + queue).
        let outcome = kerf_worker::process_one(store.as_ref(), queue.as_ref());
        assert!(matches!(outcome, kerf_worker::Outcome::Completed { .. }));

        // 3. The job is done and points at a result.
        let (code, body) = send(
            app.clone(),
            Request::builder()
                .uri(format!("/v1/jobs/{job_id}"))
                .header("x-api-key", "dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let result_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["result_id"]
            .as_u64()
            .unwrap();

        // 4. The immutable result reads back as SOUND.
        let (code, body) = send(
            app,
            Request::builder()
                .uri(format!("/v1/results/{result_id}"))
                .header("x-api-key", "dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["envelope"]["summary"]["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn tenants_cannot_read_across_the_boundary() {
        // acme submits; a key for another tenant must not see the job.
        let store: Arc<dyn Store> = Arc::new(MemStore::new());
        let queue: Arc<dyn Queue> = Arc::new(MemQueue::default());
        let keys = Arc::new(HashMap::from([
            ("acme-key".to_string(), "acme".to_string()),
            ("globex-key".to_string(), "globex".to_string()),
        ]));
        let st = AppState { store, queue, keys };
        let app = build_router(st);

        let (_, body) = send(app.clone(), verify_req(Some("acme-key"))).await;
        let job_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["job_id"]
            .as_u64()
            .unwrap();

        let (code, _) = send(
            app,
            Request::builder()
                .uri(format!("/v1/jobs/{job_id}"))
                .header("x-api-key", "globex-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND); // isolated, not leaked
    }
}
