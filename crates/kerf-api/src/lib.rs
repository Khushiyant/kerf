//! Kerf HTTP API: submit verify/diff jobs, poll them, read the immutable result, plus a browser
//! dashboard. Every request is authenticated by an `X-API-Key` header mapped to a tenant, and every
//! read is tenant-scoped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use kerf_queue::Queue;
use kerf_store::{Alert, Job, JobSpec, Store, StoredResult};

const DEFAULT_RESOLUTION_UM: i64 = 200;

/// Access level of an API key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Read-only: may read jobs / results / alerts, may not submit work.
    Reader,
    /// May submit work and register baselines, plus everything a Reader can.
    Writer,
}

/// Service counters exposed at `/metrics` (Prometheus text).
#[derive(Default)]
pub struct Metrics {
    pub verify: AtomicU64,
    pub diff: AtomicU64,
    pub baseline: AtomicU64,
}

/// Shared service state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub queue: Arc<dyn Queue>,
    /// API key → (tenant, role).
    pub keys: Arc<HashMap<String, (String, Role)>>,
    pub metrics: Arc<Metrics>,
}

/// Build the router. `/healthz` and `/readyz` are unauthenticated probes; everything under `/v1`
/// requires a valid `X-API-Key`.
pub fn build_router(state: AppState) -> Router {
    let app = Router::new()
        .route("/", get(dashboard))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }))
        .route("/metrics", get(metrics))
        .route("/v1/verify", post(post_verify))
        .route("/v1/diff", post(post_diff))
        .route("/v1/jobs", get(list_jobs))
        .route("/v1/jobs/{id}", get(get_job))
        .route("/v1/results/{id}", get(get_result))
        .route("/v1/results/{id}/diff.png", get(get_diff_png))
        .route("/v1/projects/{project}/baseline", post(put_baseline))
        .route("/v1/projects/{project}/check", post(post_check))
        .route("/v1/alerts", get(get_alerts))
        .with_state(state);
    // Span emitted at INFO so the default `info` filter keeps it (tower-http defaults to DEBUG).
    #[cfg(feature = "otel")]
    let app = app.layer(
        tower_http::trace::TraceLayer::new_for_http()
            .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::INFO)),
    );
    app
}

/// Initialize tracing: export spans over OTLP to `OTEL_EXPORTER_OTLP_ENDPOINT`
/// (default `http://localhost:4317`) and log to stderr.
#[cfg(feature = "otel")]
pub fn init_telemetry() {
    use opentelemetry::KeyValue;
    use opentelemetry_otlp::WithExportConfig;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint.clone()),
        )
        .with_trace_config(opentelemetry_sdk::trace::config().with_resource(
            opentelemetry_sdk::Resource::new(vec![KeyValue::new("service.name", "kerf")]),
        ))
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .expect("build OTLP tracer");

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();
    tracing::info!(endpoint = %endpoint, "OpenTelemetry tracing initialized (OTLP)");
}

/// Flush any buffered spans on shutdown.
#[cfg(feature = "otel")]
pub fn shutdown_telemetry() {
    opentelemetry::global::shutdown_tracer_provider();
}

#[cfg(not(feature = "otel"))]
pub fn init_telemetry() {}

#[cfg(not(feature = "otel"))]
pub fn shutdown_telemetry() {}

/// Resolve `(tenant, role)` for a request, or reject with 401.
fn authorize(headers: &HeaderMap, st: &AppState) -> Result<(String, Role), StatusCode> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    st.keys.get(key).cloned().ok_or(StatusCode::UNAUTHORIZED)
}

/// Require a Writer role, else 403.
fn require_write(role: Role) -> Result<(), StatusCode> {
    if role == Role::Writer {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn metrics(State(st): State<AppState>) -> impl IntoResponse {
    let m = &st.metrics;
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        format!(
            "# HELP kerf_jobs_submitted_total Jobs submitted, by kind.\n\
             # TYPE kerf_jobs_submitted_total counter\n\
             kerf_jobs_submitted_total{{kind=\"verify\"}} {}\n\
             kerf_jobs_submitted_total{{kind=\"diff\"}} {}\n\
             kerf_jobs_submitted_total{{kind=\"baseline\"}} {}\n",
            m.verify.load(Ordering::Relaxed),
            m.diff.load(Ordering::Relaxed),
            m.baseline.load(Ordering::Relaxed),
        ),
    )
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
    let (tenant, role) = authorize(&headers, &st)?;
    require_write(role)?;
    let res = resolution(req.resolution_um)?;
    st.metrics.verify.fetch_add(1, Ordering::Relaxed);
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
    let (tenant, role) = authorize(&headers, &st)?;
    require_write(role)?;
    let res = resolution(req.resolution_um)?;
    st.metrics.diff.fetch_add(1, Ordering::Relaxed);
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
    let (tenant, _) = authorize(&headers, &st)?;
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
    let (tenant, _) = authorize(&headers, &st)?;
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
    let (tenant, _) = authorize(&headers, &st)?;
    Ok(Json(st.store.list_jobs(&tenant)))
}

/// Register (or replace) a project's baseline golden part.
async fn put_baseline(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Json(req): Json<VerifyReq>,
) -> Result<StatusCode, StatusCode> {
    let (tenant, role) = authorize(&headers, &st)?;
    require_write(role)?;
    let res = resolution(req.resolution_um)?;
    let blob = st.store.put_blob(req.gcode.as_bytes());
    st.store.set_baseline(&tenant, &project, blob, res);
    Ok(StatusCode::NO_CONTENT)
}

/// Submit a file to be checked against a project's baseline (raises an alert on regression).
async fn post_check(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Json(req): Json<VerifyReq>,
) -> Result<(StatusCode, Json<JobAccepted>), StatusCode> {
    let (tenant, role) = authorize(&headers, &st)?;
    require_write(role)?;
    let res = resolution(req.resolution_um)?;
    st.metrics.baseline.fetch_add(1, Ordering::Relaxed);
    let blob = st.store.put_blob(req.gcode.as_bytes());
    let job = st.store.create_job(
        &tenant,
        JobSpec::Baseline {
            project,
            input: blob,
        },
        res,
    );
    st.queue.enqueue(job);
    Ok((StatusCode::ACCEPTED, Json(JobAccepted { job_id: job })))
}

async fn get_alerts(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Alert>>, StatusCode> {
    let (tenant, _) = authorize(&headers, &st)?;
    Ok(Json(st.store.list_alerts(&tenant)))
}

#[derive(Deserialize)]
struct LayerQuery {
    #[serde(default)]
    layer: usize,
}

/// Render the visual diff of a diff/baseline result to a PNG (`?layer=N` selects the layer, default 0).
async fn get_diff_png(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Query(q): Query<LayerQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let (tenant, _) = authorize(&headers, &st)?;
    let result = st
        .store
        .get_result(id)
        .filter(|r| r.tenant == tenant)
        .ok_or(StatusCode::NOT_FOUND)?;
    let job = st
        .store
        .get_job(result.job_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Recover the two inputs to diff, depending on the job kind.
    let (a_id, b_id) = match &job.spec {
        JobSpec::Diff { a, b } => (a.clone(), b.clone()),
        JobSpec::Baseline { project, input } => {
            let baseline = st
                .store
                .get_baseline(&tenant, project)
                .ok_or(StatusCode::NOT_FOUND)?;
            (input.clone(), baseline.blob)
        }
        JobSpec::Verify { .. } => return Err(StatusCode::BAD_REQUEST),
    };
    let (Some(a), Some(b)) = (st.store.get_blob(&a_id), st.store.get_blob(&b_id)) else {
        return Err(StatusCode::NOT_FOUND);
    };

    let res = job.resolution_um;
    let layer = q.layer;
    // Rendering re-runs denote (CPU-bound); keep it off the async reactor.
    let png = tokio::task::spawn_blocking(move || {
        let a = String::from_utf8_lossy(&a).into_owned();
        let b = String::from_utf8_lossy(&b).into_owned();
        kerf_render::diff_pngs_from_gcode(&a, &b, res, 512)
            .into_iter()
            .nth(layer)
            .map(|(_, png)| png)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    Ok(([(header::CONTENT_TYPE, "image/png")], png))
}

async fn dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

const DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Kerf</title>
<style>
body{font:14px system-ui;margin:2rem;max-width:60rem}
textarea{width:100%;height:8rem}
pre{background:#f4f4f4;padding:1rem;overflow:auto}
input,button{font:inherit;padding:.3rem}
table{border-collapse:collapse;width:100%;margin:.5rem 0}
th,td{border:1px solid #ddd;padding:.3rem .5rem;text-align:left;font-size:13px}
tr.regress td{color:#c22}
h2{margin-top:1.5rem}
</style></head>
<body>
<h1>Kerf</h1><p>Verify G-code by the material it deposits.</p>
<p>API key <input id="key" value="dev-key"> resolution µm <input id="res" value="200" size="5">
<button onclick="refresh()">Refresh</button></p>
<textarea id="g" placeholder="paste G-code here"></textarea>
<p><button onclick="run()">Verify</button></p>
<pre id="out">(result appears here)</pre>
<h2>Recent jobs</h2>
<table id="jobs"><thead><tr><th>id</th><th>kind</th><th>status</th><th>result</th></tr></thead><tbody></tbody></table>
<h2>Regression alerts</h2>
<table id="alerts"><thead><tr><th>id</th><th>project</th><th>IoU</th><th>message</th></tr></thead><tbody></tbody></table>
<script>
const key=()=>document.getElementById('key').value;
const H=()=>({'x-api-key':key()});
async function run(){
 const res=+document.getElementById('res').value, out=document.getElementById('out'); out.textContent='submitting...';
 let r=await fetch('/v1/verify',{method:'POST',headers:{'content-type':'application/json',...H()},body:JSON.stringify({gcode:document.getElementById('g').value,resolution_um:res})});
 if(!r.ok){out.textContent='error '+r.status;return}
 const {job_id}=await r.json();
 for(let i=0;i<50;i++){await new Promise(s=>setTimeout(s,120));
  const j=await(await fetch('/v1/jobs/'+job_id,{headers:H()})).json();
  if(j.status==='done'){const rr=await(await fetch('/v1/results/'+j.result_id,{headers:H()})).json();
   out.textContent=JSON.stringify(rr.envelope.summary,null,2)+'\n\ndigest '+rr.result_digest; refresh(); return}}
 out.textContent='timed out waiting for worker';
}
async function refresh(){
 try{
  const jobs=await(await fetch('/v1/jobs',{headers:H()})).json();
  document.querySelector('#jobs tbody').innerHTML=jobs.slice(-20).reverse()
    .map(j=>`<tr><td>${j.id}</td><td>${j.spec.kind}</td><td>${j.status}</td><td>${j.result_id??''}</td></tr>`).join('');
  const alerts=await(await fetch('/v1/alerts',{headers:H()})).json();
  document.querySelector('#alerts tbody').innerHTML=alerts.slice(-20).reverse()
    .map(a=>`<tr class=regress><td>${a.id}</td><td>${a.project}</td><td>${a.iou==null?'':a.iou.toFixed(3)}</td><td>${a.message}</td></tr>`).join('');
 }catch(e){}
}
refresh();
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
        let keys = Arc::new(HashMap::from([
            ("dev".to_string(), ("acme".to_string(), Role::Writer)),
            ("dev-ro".to_string(), ("acme".to_string(), Role::Reader)),
        ]));
        (
            AppState {
                store: store.clone(),
                queue: queue.clone(),
                keys,
                metrics: Arc::new(Metrics::default()),
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

        let (code, body) = send(app.clone(), verify_req(Some("dev"))).await;
        assert_eq!(code, StatusCode::ACCEPTED);
        let job_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["job_id"]
            .as_u64()
            .unwrap();

        let outcome = kerf_worker::process_one(store.as_ref(), queue.as_ref());
        assert!(matches!(outcome, kerf_worker::Outcome::Completed { .. }));

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

    fn json_req(
        method: &str,
        uri: &str,
        key: Option<&str>,
        body: serde_json::Value,
    ) -> Request<Body> {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(k) = key {
            b = b.header("x-api-key", k);
        }
        b.body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    #[tokio::test]
    async fn baseline_regression_surfaces_an_alert() {
        let (st, store, queue) = state();
        let app = build_router(st);

        // Register the golden part.
        let (code, _) = send(
            app.clone(),
            json_req(
                "POST",
                "/v1/projects/widget/baseline",
                Some("dev"),
                serde_json::json!({ "gcode": GCODE }),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::NO_CONTENT);

        // Submit a *changed* part for checking.
        let changed = GCODE.replace("X10 Y10", "X10 Y40");
        let (code, _) = send(
            app.clone(),
            json_req(
                "POST",
                "/v1/projects/widget/check",
                Some("dev"),
                serde_json::json!({ "gcode": changed }),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED);

        // Worker processes the check.
        assert!(matches!(
            kerf_worker::process_one(store.as_ref(), queue.as_ref()),
            kerf_worker::Outcome::Completed { .. }
        ));

        // The regression shows up as an alert.
        let (code, body) = send(
            app,
            Request::builder()
                .uri("/v1/alerts")
                .header("x-api-key", "dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let alerts: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(alerts.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn diff_result_renders_a_png() {
        let (st, store, queue) = state();
        let app = build_router(st);
        // Submit a diff of two different files.
        let changed = GCODE.replace("X10 Y10", "X10 Y40");
        let (code, body) = send(
            app.clone(),
            json_req(
                "POST",
                "/v1/diff",
                Some("dev"),
                serde_json::json!({ "a": GCODE, "b": changed }),
            ),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED);
        let job_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["job_id"]
            .as_u64()
            .unwrap();

        assert!(matches!(
            kerf_worker::process_one(store.as_ref(), queue.as_ref()),
            kerf_worker::Outcome::Completed { .. }
        ));
        let (_, jbody) = send(
            app.clone(),
            Request::builder()
                .uri(format!("/v1/jobs/{job_id}"))
                .header("x-api-key", "dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let rid = serde_json::from_str::<serde_json::Value>(&jbody).unwrap()["result_id"]
            .as_u64()
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/results/{rid}/diff.png"))
                    .header("x-api-key", "dev")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("content-type").unwrap(), "image/png");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[tokio::test]
    async fn readers_cannot_submit_but_can_read() {
        let (st, store, queue) = state();
        let app = build_router(st);
        // A reader key may not submit.
        let (code, _) = send(app.clone(), verify_req(Some("dev-ro"))).await;
        assert_eq!(code, StatusCode::FORBIDDEN);
        // A writer key submits.
        let (_, body) = send(app.clone(), verify_req(Some("dev"))).await;
        let job_id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["job_id"]
            .as_u64()
            .unwrap();
        kerf_worker::process_one(store.as_ref(), queue.as_ref());
        // The reader may read it.
        let (code, _) = send(
            app,
            Request::builder()
                .uri(format!("/v1/jobs/{job_id}"))
                .header("x-api-key", "dev-ro")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
    }

    #[tokio::test]
    async fn metrics_counts_submissions() {
        let (st, _, _) = state();
        let app = build_router(st);
        send(app.clone(), verify_req(Some("dev"))).await;
        let (code, body) = send(
            app,
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert!(
            body.contains("kerf_jobs_submitted_total{kind=\"verify\"} 1"),
            "metrics body: {body}"
        );
    }

    #[tokio::test]
    async fn tenants_cannot_read_across_the_boundary() {
        let store: Arc<dyn Store> = Arc::new(MemStore::new());
        let queue: Arc<dyn Queue> = Arc::new(MemQueue::default());
        let keys = Arc::new(HashMap::from([
            ("acme-key".to_string(), ("acme".to_string(), Role::Writer)),
            (
                "globex-key".to_string(),
                ("globex".to_string(), Role::Writer),
            ),
        ]));
        let st = AppState {
            store,
            queue,
            keys,
            metrics: Arc::new(Metrics::default()),
        };
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
        assert_eq!(code, StatusCode::NOT_FOUND);
    }
}
