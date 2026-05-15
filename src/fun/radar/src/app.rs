use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, bail};
use axum::Json;
use axum::extract::State;
use axum::routing::get;
use axum::{Router, response::IntoResponse};
use tower_http::services::ServeDir;
use tracing::info;

use crate::model::{RadarDebugCompareResponse, RadarDebugSnapshot};
use crate::reader::ReaderState;

#[derive(Clone)]
struct AppState {
    latest: crate::reader::SharedSnapshot,
    latest_compare: crate::reader::SharedCompare,
}

const MEMREADER_DEVICE_PATH: &str = "/dev/memreader";

pub async fn run() -> anyhow::Result<()> {
    ensure_memreader_device()?;

    let state = Arc::new(ReaderState::new());
    state.spawn();

    let app_state = AppState {
        latest: Arc::clone(&state.latest),
        latest_compare: Arc::clone(&state.latest_compare),
    };
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_root.join("../../..");
    let web_root = repo_root.join("assets/radar/web");
    let assets_root = repo_root.join("assets/radar");
    let addr: SocketAddr = std::env::var("FUN_RADAR_BIND")
        .unwrap_or_else(|_| "0.0.0.0:2137".to_string())
        .parse()
        .context("parse FUN_RADAR_BIND")?;

    let app = Router::new()
        .route("/json", get(api_snapshot))
        .route("/api/snapshot", get(api_snapshot))
        .route("/api/raw", get(api_snapshot))
        .route("/api/debug/compare", get(api_compare))
        .nest_service("/assets", ServeDir::new(assets_root))
        .fallback_service(ServeDir::new(web_root))
        .with_state(app_state);

    info!(%addr, "fun-radar starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn ensure_memreader_device() -> anyhow::Result<()> {
    let device_path = PathBuf::from(MEMREADER_DEVICE_PATH);
    if device_path.exists() {
        return Ok(());
    }

    let module_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../host/kmod-memreader/module");
    let module_path = module_dir.join("memreader.ko");
    if !module_path.exists() {
        let status = Command::new("make")
            .arg("-C")
            .arg(&module_dir)
            .status()
            .with_context(|| format!("failed to execute make -C {}", module_dir.display()))?;
        if !status.success() {
            bail!(
                "make -C {} failed with status {}",
                module_dir.display(),
                status
            );
        }
    }

    if !module_path.exists() {
        bail!(
            "{MEMREADER_DEVICE_PATH} is missing and kernel module not found at {}",
            module_path.display()
        );
    }

    let status = Command::new("insmod")
        .arg(&module_path)
        .status()
        .with_context(|| format!("failed to execute insmod {}", module_path.display()))?;
    if !status.success() {
        bail!(
            "insmod {} failed with status {}; run as root or load the module manually",
            module_path.display(),
            status
        );
    }
    if !device_path.exists() {
        bail!(
            "insmod {} succeeded but {} is still missing",
            module_path.display(),
            MEMREADER_DEVICE_PATH
        );
    }
    Ok(())
}

async fn api_snapshot(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.latest.read().await.clone();
    Json::<RadarDebugSnapshot>(snapshot)
}

async fn api_compare(State(state): State<AppState>) -> impl IntoResponse {
    let response = state.latest_compare.read().await.clone();
    Json::<RadarDebugCompareResponse>(response)
}
