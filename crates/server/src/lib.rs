use std::sync::Arc;

use axum::{Router, middleware::from_fn, routing::get, routing::post};
use tokio::time::Duration;

use crate::config::Config;
use crate::runtime_metrics::RuntimeMetrics;
use crate::store::ResponseStore;
use crate::upstream::UpstreamClient;
use providers::ProviderProfile;

mod auth;
pub mod codex_integration;
pub mod config;
mod responses_canonicalizer;
mod routes;
pub mod runtime_metrics;
mod sse_writer;
mod store;
mod upstream;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Arc<ResponseStore>,
    pub upstream: Arc<UpstreamClient>,
    pub profile: Arc<dyn ProviderProfile>,
    pub metrics: Arc<RuntimeMetrics>,
}

/// Build the Axum application.
pub fn app(config: Config) -> anyhow::Result<Router> {
    app_with_metrics(config, Arc::new(RuntimeMetrics::default()))
}

/// Build the Axum application with a shared runtime metrics collector.
pub fn app_with_metrics(config: Config, metrics: Arc<RuntimeMetrics>) -> anyhow::Result<Router> {
    config.validate()?;
    let store_backend: Box<dyn crate::store::ResponseStoreBackend> =
        match config.state.backend.as_str() {
            "memory" | "ram" => Box::new(crate::store::MemoryStore::new(
                config.state.ttl_seconds,
                config.state.debug_artifact_ttl_seconds,
                config.state.failed_debug_artifact_ttl_seconds,
            )),
            #[cfg(feature = "sqlite")]
            "sqlite" => {
                let db_path = config
                    .state
                    .sqlite_path
                    .clone()
                    .map(|path| crate::config::expand_tilde(&path))
                    .unwrap_or_else(crate::config::default_state_store_path);
                if let Some(parent) = db_path.parent() {
                    std::fs::create_dir_all(parent)
                        .expect("Failed to create SQLite state directory");
                }
                Box::new(
                    crate::store::SqliteStore::new(
                        &db_path,
                        config.state.ttl_seconds,
                        config.state.debug_artifact_ttl_seconds,
                        config.state.failed_debug_artifact_ttl_seconds,
                    )
                    .expect("Failed to open SQLite store"),
                )
            }
            #[cfg(not(feature = "sqlite"))]
            "sqlite" => anyhow::bail!(
                "state.backend = sqlite requires building codex-shim with the `sqlite` feature"
            ),
            other => anyhow::bail!(
                "unsupported state.backend `{}`; expected `memory`, `ram`, or `sqlite`",
                other
            ),
        };
    let store = Arc::new(ResponseStore::new(store_backend));
    let upstream = Arc::new(UpstreamClient::new(config.upstream.clone())?);
    let profile: Arc<dyn ProviderProfile> = {
        use crate::provider_profile_config::ProviderProfileConfig;
        let profile_cfg =
            config
                .provider
                .profile_config
                .clone()
                .unwrap_or_else(|| ProviderProfileConfig {
                    profile: config.provider.kind.clone(),
                    ..Default::default()
                });
        Arc::from(profile_cfg.build_profile())
    };

    // Spawn background cleanup task
    let cleanup_store = store.clone();
    let cleanup_metrics = metrics.clone();
    let cleanup_interval = config.state.cleanup_interval_seconds;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(cleanup_interval));
        loop {
            interval.tick().await;
            match cleanup_store.cleanup_expired() {
                Ok(removed) => {
                    if let Ok(size) = cleanup_store.len() {
                        cleanup_metrics.set_store_size(size);
                    }
                    if removed > 0 {
                        tracing::info!(removed, "Cleaned up expired responses/debug artifacts");
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "Failed to clean up expired response state");
                }
            }
        }
    });

    metrics.set_store_size(store.len()?);

    let state = AppState {
        config: Arc::new(config),
        store,
        upstream,
        profile,
        metrics,
    };

    let auth_config: &'static _ = {
        let c = state.config.clone();
        Box::leak(Box::new(c.server.auth.clone()))
    };

    Ok(Router::new()
        .route("/healthz", get(routes::healthz))
        .route("/v1/models", get(routes::models))
        .route("/models", get(routes::models))
        .route("/v1/responses", post(routes::create_response))
        .route("/responses", post(routes::create_response))
        .route("/v1/debug/responses", get(routes::list_responses_debug))
        .route("/debug/responses", get(routes::list_responses_debug))
        .route(
            "/v1/responses/compact",
            post(routes::compact_not_implemented),
        )
        .route("/responses/compact", post(routes::compact_not_implemented))
        .route(
            "/v1/memories/trace_summarize",
            post(routes::memories_not_implemented),
        )
        .route(
            "/memories/trace_summarize",
            post(routes::memories_not_implemented),
        )
        .route(
            "/v1/responses/{id}",
            get(routes::get_response).delete(routes::delete_response),
        )
        .route("/v1/responses/{id}/debug", get(routes::get_response_debug))
        .route("/responses/{id}/debug", get(routes::get_response_debug))
        .layer(from_fn(move |req, next| {
            auth::auth_middleware(auth_config, req, next)
        }))
        .with_state(state))
}
pub mod provider_profile_config;
