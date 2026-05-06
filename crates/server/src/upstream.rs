use std::sync::Mutex;
use std::time::{Duration, Instant};

use protocol::chat::ChatCompletionRequest;
use protocol::error::ApiError;
use reqwest::Client;

use crate::config::UpstreamConfig;

/// Cached auth token from command-based auth helper.
struct CachedToken {
    value: String,
    expires_at: Instant,
}

/// HTTP client for communicating with the upstream API.
pub struct UpstreamClient {
    client: Client,
    config: UpstreamConfig,
    cached_token: Mutex<Option<CachedToken>>,
}

impl UpstreamClient {
    pub fn new(config: UpstreamConfig) -> anyhow::Result<Self> {
        let builder = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .connect_timeout(Duration::from_secs(config.connect_timeout_seconds));

        Ok(Self {
            client: builder.build()?,
            config,
            cached_token: Mutex::new(None),
        })
    }

    /// Build a request builder with auth headers, query params, and static/env headers.
    pub async fn build_request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, ApiError> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.config.base_url, path)
        };

        let mut builder = self.client.request(method, &url);

        // Append query params via URL-safe encoding
        if !self.config.query_params.is_empty() {
            builder = builder.query(&self.config.query_params);
        }

        // Auth: priority = requires_openai_auth → auth_command → env_key
        if self.config.requires_openai_auth {
            // The OpenAI login token is handled externally (via Codex's own auth flow).
            // Here we just skip injecting a Bearer token and let the upstream handle it.
        } else if let Some(ref cmd_cfg) = self.config.auth_command {
            let token = self.get_command_token(cmd_cfg).await?;
            builder = builder.bearer_auth(token);
        } else if let Ok(key) = std::env::var(&self.config.api_key_env) {
            builder = builder.bearer_auth(key);
        }

        // Static HTTP headers
        for (k, v) in &self.config.http_headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        // Environment-var-sourced HTTP headers
        for (header_name, env_var) in &self.config.env_http_headers {
            if let Ok(val) = std::env::var(env_var) {
                builder = builder.header(header_name.as_str(), val.as_str());
            }
        }

        Ok(builder)
    }

    /// Get a token from the command-based auth helper, with caching.
    async fn get_command_token(
        &self,
        cfg: &crate::config::AuthCommandConfig,
    ) -> Result<String, ApiError> {
        // Check cached token first
        {
            let guard = self.cached_token.lock().unwrap();
            if let Some(ref cached) = *guard
                && cached.expires_at > Instant::now()
            {
                return Ok(cached.value.clone());
            }
        }

        // Execute the auth command with timeout
        let output = tokio::time::timeout(
            Duration::from_millis(cfg.timeout_ms),
            tokio::process::Command::new(&cfg.command)
                .args(&cfg.args)
                .current_dir(cfg.cwd.as_deref().unwrap_or("."))
                .output(),
        )
        .await
        .map_err(|_| ApiError::upstream_timeout())
        .and_then(|r| {
            r.map_err(|e| ApiError::upstream_error(format!("auth command failed: {e}")))
        })?;

        if !output.status.success() {
            return Err(ApiError::upstream_error(format!(
                "auth command exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if token.is_empty() {
            return Err(ApiError::upstream_error(
                "auth command returned empty token",
            ));
        }

        // Cache the token
        {
            let mut guard = self.cached_token.lock().unwrap();
            *guard = Some(CachedToken {
                value: token.clone(),
                expires_at: Instant::now() + Duration::from_millis(cfg.refresh_interval_ms),
            });
        }

        Ok(token)
    }

    /// Send a non-streaming Chat Completions request to the upstream.
    pub async fn send_chat(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<protocol::chat::ChatCompletionResponse, ApiError> {
        let mut attempt = 0;
        loop {
            let builder = self
                .build_request(reqwest::Method::POST, &self.config.chat_path)
                .await?;

            let result = builder.json(req).send().await;

            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();

                    if (200..300).contains(&status) {
                        return serde_json::from_str(&body).map_err(|e| {
                            ApiError::upstream_error(format!(
                                "Failed to parse upstream response: {e}"
                            ))
                        });
                    }

                    if status == 429 || (500..600).contains(&status) {
                        attempt += 1;
                        if attempt <= self.config.max_retries {
                            let delay = Duration::from_secs(2u64.pow(attempt));
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                    }

                    return Err(mapper::error_mapper::map_upstream_error(status, &body));
                }
                Err(e) => {
                    if e.is_timeout() {
                        return Err(ApiError::upstream_timeout());
                    }
                    attempt += 1;
                    if attempt <= self.config.max_retries {
                        let delay = Duration::from_secs(2u64.pow(attempt));
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(ApiError::upstream_error(format!("{e}")));
                }
            }
        }
    }

    /// Fetch model list from upstream.
    pub async fn fetch_models(&self) -> Result<serde_json::Value, ApiError> {
        let builder = self
            .build_request(reqwest::Method::GET, &self.config.models_path)
            .await?;

        let resp = builder
            .send()
            .await
            .map_err(|e| ApiError::upstream_error(format!("Models fetch failed: {e}")))?;

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();

        if (200..300).contains(&status) {
            serde_json::from_str(&body).map_err(|e| {
                ApiError::upstream_error(format!("Failed to parse models response: {e}"))
            })
        } else {
            Ok(serde_json::json!({
                "object": "list",
                "data": []
            }))
        }
    }
}
