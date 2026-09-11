//! `/api/*` JSON REST surface: hardware/VFO/gain control, channel lifecycle and per-channel
//! demod/volume/squelch tuning, and recording control — everything that is a request/response
//! action rather than a stream. High-rate data (spectrum bins, audio samples, telemetry
//! frames, live aircraft snapshots) never flows through here; see `crate::web::ws` for that.
//!
//! Every handler is a thin translation from an HTTP verb/body onto the exact same
//! [`DaemonState`] methods the TCP control plane (`crate::server::apply_command`) already
//! calls — this is deliberately not a second copy of that command-application logic, just a
//! different transport reaching the same seam.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use ez_proto::{
    ChannelId, ChannelSpec, DemodMode, HardwareStatus, RecordingFormat, RecordingStatus,
};

use crate::state::{ChannelMetrics, DaemonState};

/// Assembles every `/api/*` route. `Router<Arc<DaemonState>>` (state not yet attached) so
/// `crate::web::serve` can `.merge()` this alongside `crate::web::ws::router()` before calling
/// `.with_state(...)` exactly once for the combined router.
pub fn router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/hardware", get(get_hardware))
        .route(
            "/api/hardware/frequency",
            axum::routing::post(set_frequency),
        )
        .route(
            "/api/hardware/sample-rate",
            axum::routing::post(set_sample_rate),
        )
        .route("/api/hardware/gain", axum::routing::post(set_gain))
        .route("/api/channels", get(list_channels).post(create_channel))
        .route(
            "/api/channels/{id}/demod-mode",
            axum::routing::post(set_demod_mode),
        )
        .route("/api/channels/{id}/volume", axum::routing::post(set_volume))
        .route(
            "/api/channels/{id}/squelch",
            axum::routing::post(set_squelch),
        )
        .route("/api/channels/{id}/recording", get(get_recording))
        .route(
            "/api/channels/{id}/recording/start",
            axum::routing::post(start_recording),
        )
        .route(
            "/api/channels/{id}/recording/stop",
            axum::routing::post(stop_recording),
        )
        .route(
            "/api/channels/{id}/retune",
            axum::routing::post(retune_channel),
        )
        .route("/api/channels/{id}", axum::routing::delete(delete_channel))
        .route("/api/recordings", get(list_recordings))
}

/// Wraps any handler-path error as a JSON `{ "error": "..." }` body. `400 Bad Request` is the
/// right default here: every fallible [`DaemonState`] call this module makes fails only on
/// caller-supplied bad input (unknown channel id, wrong channel kind for the op, ...), never
/// on an internal/server-side fault.
pub struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.0.to_string() }));
        (StatusCode::BAD_REQUEST, body).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    uptime_sec: f64,
    hardware: HardwareStatus,
}

#[derive(Debug, Deserialize)]
struct FrequencyRequest {
    hz: u64,
}

#[derive(Debug, Deserialize)]
struct SampleRateRequest {
    hz: u32,
}

#[derive(Debug, Deserialize)]
struct GainRequest {
    db: f64,
}

#[derive(Debug, Deserialize)]
struct DemodModeRequest {
    mode: DemodMode,
}

#[derive(Debug, Deserialize)]
struct VolumeRequest {
    level: f32,
}

#[derive(Debug, Deserialize)]
struct SquelchRequest {
    db: f32,
}

#[derive(Debug, Deserialize)]
struct RecordingRequest {
    format: RecordingFormat,
}

#[derive(Debug, Deserialize)]
struct RetuneRequest {
    center_offset_hz: i64,
    bandwidth_hz: u32,
}

async fn get_status(State(state): State<Arc<DaemonState>>) -> Json<StatusResponse> {
    Json(StatusResponse {
        uptime_sec: state.uptime_sec(),
        hardware: state.hardware_status(),
    })
}

async fn get_hardware(State(state): State<Arc<DaemonState>>) -> Json<HardwareStatus> {
    Json(state.hardware_status())
}

async fn set_frequency(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<FrequencyRequest>,
) -> StatusCode {
    state.set_frequency(body.hz);
    StatusCode::NO_CONTENT
}

async fn set_sample_rate(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<SampleRateRequest>,
) -> Result<StatusCode, ApiError> {
    state.set_sample_rate(body.hz)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_gain(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<GainRequest>,
) -> Result<StatusCode, ApiError> {
    state.set_gain(body.db)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_channels(State(state): State<Arc<DaemonState>>) -> Json<Vec<ChannelMetrics>> {
    Json(state.channel_metrics())
}

/// Ensures `spec.id` exists (creating its pipeline on first use, exactly like a TCP
/// `ClientCommand::Subscribe`) and returns the resulting spec. The subscription this call
/// obtains from [`DaemonState::subscribe`] is intentionally dropped immediately: the pipeline
/// itself is kept alive by `DaemonState`'s own channel table regardless of subscriber count
/// (see the `crate::state` module doc comment), so a later `/ws/stream/{kind}/{id}` connection
/// (or a TCP client) attaching its own live handle via `subscribe_existing` sees the channel
/// already warm rather than paying pipeline-startup latency.
async fn create_channel(
    State(state): State<Arc<DaemonState>>,
    Json(spec): Json<ChannelSpec>,
) -> Result<(StatusCode, Json<ChannelSpec>), ApiError> {
    let response_spec = spec.clone();
    state.subscribe(spec)?;
    Ok((StatusCode::CREATED, Json(response_spec)))
}

async fn set_demod_mode(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
    Json(body): Json<DemodModeRequest>,
) -> Result<StatusCode, ApiError> {
    state.set_demod_mode(id, body.mode)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_volume(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
    Json(body): Json<VolumeRequest>,
) -> Result<StatusCode, ApiError> {
    state.set_volume(id, body.level)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_squelch(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
    Json(body): Json<SquelchRequest>,
) -> Result<StatusCode, ApiError> {
    state.set_squelch(id, body.db)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_recording(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
    Json(body): Json<RecordingRequest>,
) -> Result<Json<RecordingStatus>, ApiError> {
    Ok(Json(state.start_recording(id, body.format)?))
}

async fn stop_recording(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
) -> Result<Json<RecordingStatus>, ApiError> {
    Ok(Json(state.stop_recording(id)?))
}

async fn get_recording(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
) -> Result<Json<RecordingStatus>, StatusCode> {
    state
        .recording_status(id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_recordings(State(state): State<Arc<DaemonState>>) -> Json<Vec<RecordingStatus>> {
    Json(state.recording_statuses())
}

async fn retune_channel(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
    Json(body): Json<RetuneRequest>,
) -> Result<StatusCode, ApiError> {
    state.retune(id, body.center_offset_hz, body.bandwidth_hz)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_channel(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<ChannelId>,
) -> Result<StatusCode, ApiError> {
    state.remove_channel(id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use crate::hardware::synthetic::SyntheticSource;
    use crate::ingest;
    use ez_proto::PipelineKind;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use tokio::net::TcpListener;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ez-daemon-api-test-{label}-{}-{n}",
            std::process::id()
        ))
    }

    /// Spins up a real HTTP listener serving just the `/api/*` router (no WS, no static
    /// files) against a fresh synthetic-source [`DaemonState`], mirroring how
    /// `crate::server`'s own tests exercise the TCP path over a real socket rather than a
    /// mocked service.
    async fn spawn_test_api() -> String {
        let bus = SampleBus::new();
        let running = Arc::new(AtomicBool::new(true));
        let (hardware, _ingest_thread) =
            ingest::spawn(Box::new(SyntheticSource::default()), bus.clone(), running).unwrap();
        let state = Arc::new(DaemonState::new(
            bus,
            hardware,
            100_000_000,
            2_000_000,
            unique_temp_dir("state"),
        ));

        let app = router().with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });
        format!("http://{addr}")
    }

    fn spectrum_spec(id: ChannelId) -> ChannelSpec {
        ChannelSpec {
            id,
            center_offset_hz: 0,
            bandwidth_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        }
    }

    fn audio_spec(id: ChannelId) -> ChannelSpec {
        ChannelSpec {
            id,
            center_offset_hz: 10_000,
            bandwidth_hz: 200_000,
            kind: PipelineKind::Audio,
            demod_mode: Some(DemodMode::Fm),
        }
    }

    #[tokio::test]
    async fn status_reports_uptime_and_hardware() {
        let base = spawn_test_api().await;
        let resp = reqwest::get(format!("{base}/api/status")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["uptime_sec"].as_f64().unwrap() >= 0.0);
        assert!(body["hardware"]["source_kind"].is_string());
    }

    #[tokio::test]
    async fn create_channel_then_list_channels_shows_it() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();

        let created = client
            .post(format!("{base}/api/channels"))
            .json(&spectrum_spec(1))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), 201);

        let listed: Vec<ChannelMetrics> = reqwest::get(format!("{base}/api/channels"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].spec.id, 1);
    }

    #[tokio::test]
    async fn set_frequency_returns_no_content() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base}/api/hardware/frequency"))
            .json(&serde_json::json!({ "hz": 105_000_000u64 }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
    }

    #[tokio::test]
    async fn operating_on_an_unknown_channel_returns_bad_request_with_error_body() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base}/api/channels/999/volume"))
            .json(&serde_json::json!({ "level": 1.0 }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("unknown channel"));
    }

    #[tokio::test]
    async fn recording_start_and_stop_round_trip() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();
        client
            .post(format!("{base}/api/channels"))
            .json(&audio_spec(2))
            .send()
            .await
            .unwrap();

        let started: RecordingStatus = client
            .post(format!("{base}/api/channels/2/recording/start"))
            .json(&serde_json::json!({ "format": "Cf32" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(started.active);

        let status: RecordingStatus = reqwest::get(format!("{base}/api/channels/2/recording"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(status.active);

        let stopped: RecordingStatus = client
            .post(format!("{base}/api/channels/2/recording/stop"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!stopped.active);
    }

    #[tokio::test]
    async fn get_recording_status_404s_for_unknown_channel() {
        let base = spawn_test_api().await;
        let resp = reqwest::get(format!("{base}/api/channels/12345/recording"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn retune_moves_an_existing_channel() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();
        client
            .post(format!("{base}/api/channels"))
            .json(&audio_spec(2))
            .send()
            .await
            .unwrap();

        let resp = client
            .post(format!("{base}/api/channels/2/retune"))
            .json(&serde_json::json!({ "center_offset_hz": 300_000, "bandwidth_hz": 100_000 }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);

        let listed: Vec<ChannelMetrics> = reqwest::get(format!("{base}/api/channels"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let spec = listed
            .iter()
            .find(|c| c.spec.id == 2)
            .expect("channel present");
        assert_eq!(spec.spec.center_offset_hz, 300_000);
        assert_eq!(spec.spec.bandwidth_hz, 100_000);
    }

    #[tokio::test]
    async fn delete_channel_removes_it_from_the_list() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();
        client
            .post(format!("{base}/api/channels"))
            .json(&audio_spec(3))
            .send()
            .await
            .unwrap();

        let resp = client
            .delete(format!("{base}/api/channels/3"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);

        let listed: Vec<ChannelMetrics> = reqwest::get(format!("{base}/api/channels"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!listed.iter().any(|c| c.spec.id == 3));
    }

    #[tokio::test]
    async fn delete_unknown_channel_returns_bad_request() {
        let base = spawn_test_api().await;
        let client = reqwest::Client::new();
        let resp = client
            .delete(format!("{base}/api/channels/999"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
    }
}
