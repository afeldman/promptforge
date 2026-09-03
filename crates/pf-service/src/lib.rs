//! pf-service — lokaler HTTP-Service (Spec §12/§13).
//!
//! Nutzt dieselbe Engine wie CLI/TUI (keine doppelte Business-Logik);
//! eine spätere GUI ist nur ein weiterer Client dieser API.
//!
//! Endpunkte (Details: docs/api.md):
//!   POST /v1/compile   {intent} → Long+Optimized+Report
//!   POST /v1/optimize  {ir, long_prompt, feedback?} → optimierter Prompt
//!   POST /v1/verify    {ir, long_prompt, optimized_prompt} → VerificationReport
//!   POST /v1/execute   {prompt} → LLM-Antwort
//!   GET  /v1/health    → Status

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pf_core::config::AppConfig;
use pf_core::error::{ErrorKind, Result, err};
use pf_engine::{Engine, StageEvent};

/// App-State: Engine (Arc, geteilt über Requests).
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub cfg: AppConfig,
}

/// Startet den Service (blockierend) auf `host:port` aus der Config.
pub async fn run_server(state: AppState) -> Result<()> {
    let addr = format!("{}:{}", state.cfg.service.host, state.cfg.service.port);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| err(ErrorKind::Io, format!("Bind {addr}: {e}")))?;
    tracing::info!(addr = %addr, "prompt-forge service listening");
    println!("prompt-forge service: http://{addr}  (Ctrl-C zum Beenden)");
    axum::serve(listener, app)
        .await
        .map_err(|e| err(ErrorKind::Io, format!("Service-Fehler: {e}")))
}

/// Blockierender Einstieg für CLI (`prompt-forge serve`).
pub fn serve(cfg: &AppConfig, engine: Arc<Engine>) -> Result<()> {
    let state = AppState {
        engine,
        cfg: cfg.clone(),
    };
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| err(ErrorKind::Io, format!("tokio runtime: {e}")))?;
    rt.block_on(run_server(state))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/compile", post(compile_handler))
        .route("/v1/optimize", post(optimize_handler))
        .route("/v1/verify", post(verify_handler))
        .route("/v1/execute", post(execute_handler))
        .route("/v1/health", get(health_handler))
        .with_state(state)
}

// --- Handler ---

async fn compile_handler(
    State(state): State<AppState>,
    Json(body): Json<CompileRequest>,
) -> Response {
    let engine = Arc::clone(&state.engine);
    let outcome = tokio::task::spawn_blocking(move || engine.compile(&body.intent, None))
        .await
        .map_err(|e| err(ErrorKind::Io, format!("Task-Fehler: {e}")))
        .and_then(|r| r);
    match outcome {
        Ok(o) => {
            let payload = serde_json::json!({
                "request_id": o.request_id,
                "llm_used": o.llm_used,
                "stages": o.stages_done,
                "long_prompt": o.long_prompt,
                "optimized_prompt": o.optimized_prompt,
                "token_report": o.token_report,
                "verification": o.verification,
            });
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn optimize_handler(
    State(state): State<AppState>,
    Json(body): Json<OptimizeRequest>,
) -> Response {
    let engine = Arc::clone(&state.engine);
    let outcome = tokio::task::spawn_blocking(move || {
        let ir = pf_core::ir::PromptIr::from_json(&serde_json::to_string(&body.ir)?)?;
        let mut cb: Option<Box<dyn FnMut(StageEvent)>> = None;
        engine.optimize_and_verify(
            &ir,
            &body.long_prompt,
            &pf_core::path::request_id(),
            &mut cb,
        )
    })
    .await
    .map_err(|e| err(ErrorKind::Io, format!("Task-Fehler: {e}")))
    .and_then(|r| r);
    match outcome {
        Ok((optimized, verification, events)) => {
            let payload = serde_json::json!({
                "optimized_prompt": optimized,
                "verification": verification,
                "optimizer_passes": events.iter().map(|e| e.pass.clone()).collect::<Vec<_>>(),
            });
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn verify_handler(
    State(state): State<AppState>,
    Json(body): Json<VerifyRequest>,
) -> Response {
    let engine = Arc::clone(&state.engine);
    let outcome = tokio::task::spawn_blocking(move || {
        let ir = pf_core::ir::PromptIr::from_json(&serde_json::to_string(&body.ir)?)?;
        let mut cb: Option<Box<dyn FnMut(StageEvent)>> = None;
        engine.verify_pair(
            &ir,
            &body.long_prompt,
            &body.optimized_prompt,
            &pf_core::path::request_id(),
            &mut cb,
        )
    })
    .await
    .map_err(|e| err(ErrorKind::Io, format!("Task-Fehler: {e}")))
    .and_then(|r| r);
    match outcome {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(e) => error_response(e),
    }
}

async fn execute_handler(
    State(state): State<AppState>,
    Json(body): Json<ExecuteRequest>,
) -> Response {
    let engine = Arc::clone(&state.engine);
    let outcome = tokio::task::spawn_blocking(move || {
        pf_engine::pipeline::execute_prompt(&engine, &body.prompt, None)
    })
    .await
    .map_err(|e| err(ErrorKind::Io, format!("Task-Fehler: {e}")))
    .and_then(|r| r);
    match outcome {
        Ok(resp) => {
            let payload = serde_json::json!({
                "content": resp.content,
                "model": resp.model,
                "finish_reason": resp.finish_reason,
                "usage": resp.usage,
                "duration_ms": resp.duration_ms,
            });
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(e) => error_response(e),
    }
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "prompt-forge",
        "version": pf_core::VERSION,
        "provider": format!("{:?}", state.cfg.effective_provider()),
        "model": state.cfg.llm.model,
    }))
}

// --- Request-/Fehler-Typen ---

#[derive(Debug, serde::Deserialize)]
pub struct CompileRequest {
    pub intent: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct OptimizeRequest {
    /// Prompt-IR als Objekt.
    pub ir: serde_json::Value,
    pub long_prompt: String,
    #[serde(default)]
    pub feedback: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct VerifyRequest {
    pub ir: serde_json::Value,
    pub long_prompt: String,
    pub optimized_prompt: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ExecuteRequest {
    pub prompt: String,
}

/// JSON-Fehler-Envelope (Spec §19): {kind, message, retryable}.
fn error_response(e: pf_core::PfError) -> Response {
    let status = match e.kind {
        ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorKind::Configuration => StatusCode::BAD_REQUEST,
        ErrorKind::Verification | ErrorKind::Optimization => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorKind::Provider | ErrorKind::Authentication | ErrorKind::Model | ErrorKind::Timeout => {
            StatusCode::BAD_GATEWAY
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    tracing::warn!(kind = %e.kind, "request failed: {e}");
    (
        status,
        Json(serde_json::to_value(&e).unwrap_or(serde_json::json!({"kind": "error"}))),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf_core::config::VerifyConfig;
    use pf_core::token::HeuristicTokenizer;
    use pf_engine::EngineConfig;
    use pf_engine::mock::MockBridge;

    #[test]
    fn error_status_mapping() {
        assert_eq!(
            error_status(ErrorKind::Verification),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            error_status(ErrorKind::InvalidInput),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            error_status(ErrorKind::Bridge),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn error_status(k: ErrorKind) -> StatusCode {
        match k {
            ErrorKind::InvalidInput | ErrorKind::Configuration => StatusCode::BAD_REQUEST,
            ErrorKind::Verification | ErrorKind::Optimization => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorKind::Provider
            | ErrorKind::Authentication
            | ErrorKind::Model
            | ErrorKind::Timeout => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[tokio::test]
    async fn health_endpoint_ok() {
        let cfg = AppConfig::load(Some(std::path::Path::new("/tmp/pf-svc-test"))).unwrap();
        let engine = Arc::new(Engine::new(
            Box::new(MockBridge::new()),
            Arc::new(HeuristicTokenizer),
            EngineConfig {
                llm: Default::default(),
                verify: VerifyConfig::default(),
            },
            pf_core::config::ProviderKind::Mock,
        ));
        let app = router(AppState { engine, cfg });
        use tower::ServiceExt;
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn compile_endpoint_returns_optimized() {
        let cfg = AppConfig::load(Some(std::path::Path::new("/tmp/pf-svc-test"))).unwrap();
        let engine = Arc::new(Engine::new(
            Box::new(MockBridge::new()),
            Arc::new(HeuristicTokenizer),
            EngineConfig {
                llm: Default::default(),
                verify: VerifyConfig::default(),
            },
            pf_core::config::ProviderKind::Mock,
        ));
        let app = router(AppState { engine, cfg });
        use tower::ServiceExt;
        let body =
            serde_json::json!({ "intent": "Analysiere fünf Papers und vergleiche die Methoden" });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/compile")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
