//! FreeCodex multi-provider free model pool with random routing and retry.
//!
//! Users do not choose models — all traffic defaults to the free pool.
//! `ModelTier::Premium` + `PREMIUM_MODEL_PREFIX` are reserved for future
//! credit-redeemed advanced models.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::protocol_proxy::{UpstreamProxyResponse, chat_completions_url};

static ROUTE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum upstream attempts before surfacing the last provider error.
pub const MAX_ROUTE_ATTEMPTS: u32 = 1000;

pub const DEFAULT_MODEL_ID: &str = "freecodex";
/// Canonical Codex `model_provider` id for FreeCodex sessions.
pub const DEFAULT_PROVIDER_ID: &str = "freecodex";
/// Reserved prefix for future credit-redeemed premium models.
pub const PREMIUM_MODEL_PREFIX: &str = "freecodex-premium-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    /// Default free pool (fast / daily).
    Daily,
    /// Heavier free pool (reasoning-class endpoints).
    Reasoning,
    /// Reserved for future credit-redeemed advanced routing.
    Premium,
}

#[derive(Debug, Clone)]
pub struct FreeModelEndpoint {
    pub provider: &'static str,
    pub base_url: &'static str,
    pub model: &'static str,
    pub tier: ModelTier,
}

#[derive(Debug, Clone)]
pub struct FreeCodexRoutingPlan {
    pub tier: ModelTier,
    /// Credits charged per request. Zero while the product is fully free.
    pub required_credits: i64,
    pub inject_downgrade_warning: bool,
}

const OPENCODE_ZEN_BASE: &str = "https://opencode.ai/zen/v1";
const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const NVIDIA_NIM_BASE: &str = "https://integrate.api.nvidia.com/v1";

const ALL_ENDPOINTS: &[FreeModelEndpoint] = &[
    FreeModelEndpoint {
        provider: "opencode",
        base_url: OPENCODE_ZEN_BASE,
        model: "deepseek-v4-flash-free",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "opencode",
        base_url: OPENCODE_ZEN_BASE,
        model: "mimo-v2.5-free",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "opencode",
        base_url: OPENCODE_ZEN_BASE,
        model: "north-mini-code-free",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "opencode",
        base_url: OPENCODE_ZEN_BASE,
        model: "big-pickle",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "opencode",
        base_url: OPENCODE_ZEN_BASE,
        model: "nemotron-3-ultra-free",
        tier: ModelTier::Reasoning,
    },
    FreeModelEndpoint {
        provider: "openrouter",
        base_url: OPENROUTER_BASE,
        model: "cohere/north-mini-code:free",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "openrouter",
        base_url: OPENROUTER_BASE,
        model: "nex-agi/nex-n2-pro:free",
        tier: ModelTier::Reasoning,
    },
    FreeModelEndpoint {
        provider: "openrouter",
        base_url: OPENROUTER_BASE,
        model: "nvidia/nemotron-3-ultra-550b-a55b:free",
        tier: ModelTier::Reasoning,
    },
    FreeModelEndpoint {
        provider: "nvidia",
        base_url: NVIDIA_NIM_BASE,
        model: "nvidia/nemotron-3-nano-30b-a3b",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "nvidia",
        base_url: NVIDIA_NIM_BASE,
        model: "deepseek-ai/deepseek-v4-flash",
        tier: ModelTier::Daily,
    },
    FreeModelEndpoint {
        provider: "nvidia",
        base_url: NVIDIA_NIM_BASE,
        model: "meta/llama-3.3-70b-instruct",
        tier: ModelTier::Reasoning,
    },
    FreeModelEndpoint {
        provider: "nvidia",
        base_url: NVIDIA_NIM_BASE,
        model: "nvidia/nemotron-3-ultra-550b-a55b",
        tier: ModelTier::Reasoning,
    },
];

const NO_PROVIDERS_ERROR: &str = "FreeCodex model pool is temporarily unavailable.";

/// Internal helper — upstream GPT ids are silently replaced, never user-facing.
pub fn is_gpt_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    if m.is_empty() || m == DEFAULT_MODEL_ID || m.starts_with("freecodex-") {
        return false;
    }
    if m.contains("gpt-oss") {
        return false;
    }
    m.contains("gpt-")
        || m.contains("/gpt-")
        || m.starts_with("gpt")
        || m.contains("openai/gpt")
        || m.contains("text-davinci")
        || m.contains("chatgpt")
        || m.contains("gpt-3.5-codex")
        || m.contains("gpt-5")
        || m.contains("gpt-4")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
}

pub fn plan_freecodex_route(req_model: &str, _balance: i64) -> anyhow::Result<FreeCodexRoutingPlan> {
    // Future: credit-redeemed premium models (extensibility hook).
    if req_model.starts_with(PREMIUM_MODEL_PREFIX) {
        return Ok(FreeCodexRoutingPlan {
            tier: ModelTier::Premium,
            required_credits: 0,
            inject_downgrade_warning: false,
        });
    }

    // All requests are free for now. Tier only affects which pool we draw from.
    let tier = if req_model == "freecodex-reasoning" {
        ModelTier::Reasoning
    } else {
        ModelTier::Daily
    };

    Ok(FreeCodexRoutingPlan {
        tier,
        required_credits: 0,
        inject_downgrade_warning: false,
    })
}

pub fn inject_downgrade_warning(messages: &mut Value) {
    if let Some(arr) = messages.as_array_mut() {
        arr.insert(
            0,
            serde_json::json!({
                "role": "system",
                "content": "IMPORTANT: You MUST begin your response with exactly this text: '⚠️ **算力告警**：当前 Compute Credits 积分偏低，本次对话已自动降级切换至节流模式。'\n\n"
            }),
        );
    }
}

fn tier_for_routing(tier: ModelTier) -> ModelTier {
    match tier {
        ModelTier::Premium => ModelTier::Reasoning,
        other => other,
    }
}

pub fn available_endpoints(tier: ModelTier) -> Vec<FreeModelEndpoint> {
    let tier = tier_for_routing(tier);
    ALL_ENDPOINTS
        .iter()
        .filter(|ep| ep.tier == tier && !is_gpt_model(ep.model))
        .filter(|ep| crate::freecodex_provider_keys::provider_key(ep.provider).is_some())
        .cloned()
        .collect()
}

fn shuffle_endpoints(mut endpoints: Vec<FreeModelEndpoint>) -> Vec<FreeModelEndpoint> {
    if endpoints.len() <= 1 {
        return endpoints;
    }
    let seed = ROUTE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ^ SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
    for i in (1..endpoints.len()).rev() {
        let j = (seed.wrapping_mul(6364136223846793005).wrapping_add(i as u64) as usize) % (i + 1);
        endpoints.swap(i, j);
    }
    endpoints
}

fn should_failover_status(status: u16) -> bool {
    matches!(
        status,
        400 | 401 | 403 | 404 | 408 | 409 | 422 | 423 | 429 | 500 | 502 | 503 | 504 | 520 | 529
    )
}

fn route_retry_delay(attempt: u32) -> Duration {
    const BASE_MS: u64 = 250;
    const STEP_MS: u64 = 75;
    const CAP_MS: u64 = 6_000;
    const JITTER_MASK: u64 = 255;

    let growth = attempt.saturating_sub(1) as u64;
    let delay_ms = (BASE_MS + growth.saturating_mul(STEP_MS)).min(CAP_MS);
    let jitter = ROUTE_COUNTER.fetch_add(1, Ordering::Relaxed) & JITTER_MASK;
    Duration::from_millis(delay_ms + jitter)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamRouteControl {
    Completed,
    RetryBeforeOutput(String),
}

#[derive(Debug, Clone, Copy)]
pub enum StreamRelayMode<'a> {
    ChatCompletions,
    ResponsesConvert { original_request: &'a Value },
}

pub async fn route_chat_completions_stream(
    request_json: &mut Value,
    tier: ModelTier,
    user_agent: &str,
    stream: &mut tokio::net::TcpStream,
    relay_mode: StreamRelayMode<'_>,
) -> anyhow::Result<()> {
    let mut endpoints = shuffle_endpoints(available_endpoints(tier));
    if endpoints.is_empty() {
        anyhow::bail!(NO_PROVIDERS_ERROR);
    }

    let mut last_error = String::new();
    let client = crate::http_client::proxied_client(user_agent)?;
    let mut attempt = 0u32;
    let mut endpoint_index = 0usize;

    loop {
        attempt += 1;
        if endpoint_index >= endpoints.len() {
            endpoints = shuffle_endpoints(available_endpoints(tier));
            endpoint_index = 0;
            if endpoints.is_empty() {
                anyhow::bail!(NO_PROVIDERS_ERROR);
            }
        }

        let endpoint = &endpoints[endpoint_index];
        endpoint_index += 1;
        let is_last_attempt = attempt >= MAX_ROUTE_ATTEMPTS;

        let api_key = crate::freecodex_provider_keys::provider_key(endpoint.provider)
            .unwrap_or_default();
        request_json["model"] = Value::String(endpoint.model.to_string());
        crate::protocol_proxy::normalize_chat_request_for_upstream(request_json);

        let url = chat_completions_url(endpoint.base_url);
        let result = client
            .post(&url)
            .bearer_auth(api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&request_json)
            .send()
            .await;

        match result {
            Ok(response) => {
                let status_code = response.status().as_u16();
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                if status_code >= 400 {
                    let body_preview = response.text().await.unwrap_or_default();
                    last_error = format!(
                        "{} {} -> HTTP {}: {}",
                        endpoint.provider,
                        endpoint.model,
                        status_code,
                        truncate(&body_preview, 256)
                    );
                    if is_last_attempt || !should_failover_status(status_code) {
                        anyhow::bail!(
                            "All model providers failed after {MAX_ROUTE_ATTEMPTS} attempts. Last error: {last_error}"
                        );
                    }
                    let delay = route_retry_delay(attempt);
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "freecodex.route_retry",
                        serde_json::json!({
                            "provider": endpoint.provider,
                            "model": endpoint.model,
                            "attempt": attempt,
                            "max_attempts": MAX_ROUTE_ATTEMPTS,
                            "status": status_code,
                            "delay_ms": delay.as_millis(),
                            "error": last_error,
                            "phase": "stream_preflight",
                        }),
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }

                let upstream = UpstreamProxyResponse {
                    status_code,
                    is_stream: true,
                    content_type,
                    response,
                };
                match relay_stream_to_client(stream, relay_mode, upstream).await? {
                    StreamRouteControl::Completed => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "freecodex.route_stream_ok",
                            serde_json::json!({
                                "provider": endpoint.provider,
                                "model": endpoint.model,
                                "attempt": attempt,
                            }),
                        );
                        return Ok(());
                    }
                    StreamRouteControl::RetryBeforeOutput(reason) => {
                        last_error = format!(
                            "{} {} -> stream retry: {}",
                            endpoint.provider, endpoint.model, reason
                        );
                        if is_last_attempt {
                            anyhow::bail!(
                                "All model providers failed after {MAX_ROUTE_ATTEMPTS} attempts. Last error: {last_error}"
                            );
                        }
                        let delay = route_retry_delay(attempt);
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "freecodex.route_retry",
                            serde_json::json!({
                                "provider": endpoint.provider,
                                "model": endpoint.model,
                                "attempt": attempt,
                                "max_attempts": MAX_ROUTE_ATTEMPTS,
                                "delay_ms": delay.as_millis(),
                                "error": last_error,
                                "phase": "stream_body",
                            }),
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
            Err(error) => {
                last_error = format!(
                    "{} {} -> network error: {error}",
                    endpoint.provider, endpoint.model
                );
                if is_last_attempt {
                    anyhow::bail!(
                        "All model providers failed after {MAX_ROUTE_ATTEMPTS} attempts. Last error: {last_error}"
                    );
                }
                let delay = route_retry_delay(attempt);
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "freecodex.route_retry",
                    serde_json::json!({
                        "provider": endpoint.provider,
                        "model": endpoint.model,
                        "attempt": attempt,
                        "max_attempts": MAX_ROUTE_ATTEMPTS,
                        "delay_ms": delay.as_millis(),
                        "error": last_error,
                        "phase": "stream_network",
                    }),
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn relay_stream_to_client(
    stream: &mut tokio::net::TcpStream,
    relay_mode: StreamRelayMode<'_>,
    upstream: UpstreamProxyResponse,
) -> anyhow::Result<StreamRouteControl> {
    match relay_mode {
        StreamRelayMode::ChatCompletions => {
            relay_passthrough_sse(
                stream,
                upstream,
                "200 OK",
                "text/event-stream; charset=utf-8",
            )
            .await
        }
        StreamRelayMode::ResponsesConvert { original_request } => {
            relay_responses_converted_sse(stream, upstream, original_request).await
        }
    }
}

async fn relay_passthrough_sse(
    stream: &mut tokio::net::TcpStream,
    upstream: UpstreamProxyResponse,
    status_ok: &str,
    content_type: &str,
) -> anyhow::Result<StreamRouteControl> {
    let mut bytes_stream = upstream.response.bytes_stream();
    let mut output_started = false;

    while let Some(chunk) = bytes_stream.next().await {
        match chunk {
            Ok(bytes) => {
                if bytes.is_empty() {
                    continue;
                }
                if !output_started {
                    write_http_stream_headers(stream, status_ok, content_type).await?;
                    output_started = true;
                }
                stream.write_all(&bytes).await?;
            }
            Err(error) => {
                if !output_started {
                    return Ok(StreamRouteControl::RetryBeforeOutput(format!(
                        "Stream error: {error}"
                    )));
                }
                return Ok(StreamRouteControl::Completed);
            }
        }
    }

    if !output_started {
        return Ok(StreamRouteControl::RetryBeforeOutput(
            "empty upstream stream".to_string(),
        ));
    }
    Ok(StreamRouteControl::Completed)
}

async fn relay_responses_converted_sse(
    stream: &mut tokio::net::TcpStream,
    upstream: UpstreamProxyResponse,
    original_request: &Value,
) -> anyhow::Result<StreamRouteControl> {
    let mut converter =
        crate::protocol_proxy::ChatSseToResponsesConverter::with_request(original_request);
    let mut bytes_stream = upstream.response.bytes_stream();
    let mut output_started = false;

    while let Some(chunk) = bytes_stream.next().await {
        match chunk {
            Ok(bytes) => {
                let converted = converter.push_bytes(&bytes);
                if converted.is_empty() {
                    continue;
                }
                if !output_started {
                    write_http_stream_headers(
                        stream,
                        "200 OK",
                        "text/event-stream; charset=utf-8",
                    )
                    .await?;
                    output_started = true;
                }
                stream.write_all(&converted).await?;
            }
            Err(error) => {
                if !output_started {
                    return Ok(StreamRouteControl::RetryBeforeOutput(format!(
                        "Stream error: {error}"
                    )));
                }
                let failed = converter.fail(
                    format!("Stream error: {error}"),
                    Some("stream_error".to_string()),
                );
                if !failed.is_empty() {
                    stream.write_all(&failed).await?;
                }
                return Ok(StreamRouteControl::Completed);
            }
        }
    }

    if !output_started {
        return Ok(StreamRouteControl::RetryBeforeOutput(
            "empty upstream stream".to_string(),
        ));
    }

    let tail = converter.finish();
    if !tail.is_empty() {
        stream.write_all(&tail).await?;
    }
    Ok(StreamRouteControl::Completed)
}

async fn write_http_stream_headers(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

pub async fn route_chat_completions(
    request_json: &mut Value,
    tier: ModelTier,
    user_agent: &str,
) -> anyhow::Result<UpstreamProxyResponse> {
    let mut endpoints = shuffle_endpoints(available_endpoints(tier));
    if endpoints.is_empty() {
        anyhow::bail!(NO_PROVIDERS_ERROR);
    }

    let is_stream = request_json
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut last_error = String::new();
    let client = crate::http_client::proxied_client(user_agent)?;
    let mut attempt = 0u32;
    let mut endpoint_index = 0usize;

    loop {
        attempt += 1;
        if endpoint_index >= endpoints.len() {
            endpoints = shuffle_endpoints(available_endpoints(tier));
            endpoint_index = 0;
            if endpoints.is_empty() {
                anyhow::bail!(NO_PROVIDERS_ERROR);
            }
        }

        let endpoint = &endpoints[endpoint_index];
        endpoint_index += 1;
        let is_last_attempt = attempt >= MAX_ROUTE_ATTEMPTS;

        let api_key = crate::freecodex_provider_keys::provider_key(endpoint.provider)
            .unwrap_or_default();
        request_json["model"] = Value::String(endpoint.model.to_string());
        crate::protocol_proxy::normalize_chat_request_for_upstream(request_json);

        let url = chat_completions_url(endpoint.base_url);
        let result = client
            .post(&url)
            .bearer_auth(api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&request_json)
            .send()
            .await;

        match result {
            Ok(response) => {
                let status_code = response.status().as_u16();
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                if status_code < 400 || is_last_attempt || !should_failover_status(status_code) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        if status_code < 400 {
                            "freecodex.route_ok"
                        } else {
                            "freecodex.route_exhausted"
                        },
                        serde_json::json!({
                            "provider": endpoint.provider,
                            "model": endpoint.model,
                            "attempt": attempt,
                            "status": status_code,
                            "max_attempts": MAX_ROUTE_ATTEMPTS,
                        }),
                    );
                    return Ok(UpstreamProxyResponse {
                        status_code,
                        is_stream: is_stream || content_type.contains("text/event-stream"),
                        content_type,
                        response,
                    });
                }

                let body_preview = response.text().await.unwrap_or_default();
                last_error = format!(
                    "{} {} -> HTTP {}: {}",
                    endpoint.provider,
                    endpoint.model,
                    status_code,
                    truncate(&body_preview, 256)
                );
                let delay = route_retry_delay(attempt);
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "freecodex.route_retry",
                    serde_json::json!({
                        "provider": endpoint.provider,
                        "model": endpoint.model,
                        "attempt": attempt,
                        "max_attempts": MAX_ROUTE_ATTEMPTS,
                        "status": status_code,
                        "delay_ms": delay.as_millis(),
                        "error": last_error,
                    }),
                );
                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                last_error = format!(
                    "{} {} -> network error: {error}",
                    endpoint.provider, endpoint.model
                );
                if is_last_attempt {
                    anyhow::bail!(
                        "All model providers failed after {MAX_ROUTE_ATTEMPTS} attempts. Last error: {last_error}"
                    );
                }
                let delay = route_retry_delay(attempt);
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "freecodex.route_retry",
                    serde_json::json!({
                        "provider": endpoint.provider,
                        "model": endpoint.model,
                        "attempt": attempt,
                        "max_attempts": MAX_ROUTE_ATTEMPTS,
                        "delay_ms": delay.as_millis(),
                        "error": last_error,
                    }),
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

pub fn single_model_list_json() -> serde_json::Value {
    serde_json::json!({
        "object": "list",
        "data": [{
            "id": DEFAULT_MODEL_ID,
            "object": "model",
            "created": 1686935002,
            "owned_by": "system",
            "name": "Assistant"
        }]
    })
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_upstream_gpt_models_internally() {
        assert!(is_gpt_model("gpt-5.5"));
        assert!(!is_gpt_model(DEFAULT_MODEL_ID));
        assert!(!is_gpt_model("freecodex-reasoning"));
    }

    #[test]
    fn default_route_is_free_daily_pool() {
        let plan = plan_freecodex_route("gpt-5.5", 0).unwrap();
        assert_eq!(plan.tier, ModelTier::Daily);
        assert_eq!(plan.required_credits, 0);
    }

    #[test]
    fn premium_prefix_reserved_for_future_redemption() {
        let plan = plan_freecodex_route("freecodex-premium-opus", 100).unwrap();
        assert_eq!(plan.tier, ModelTier::Premium);
    }

    #[test]
    fn single_model_list_hides_provider_catalog() {
        let json = single_model_list_json();
        assert_eq!(json["data"].as_array().unwrap().len(), 1);
        assert_eq!(json["data"][0]["id"], DEFAULT_MODEL_ID);
        assert_eq!(json["data"][0]["name"], "Assistant");
    }

    #[test]
    fn shuffle_preserves_all_endpoints() {
        let sample = vec![
            ALL_ENDPOINTS[0].clone(),
            ALL_ENDPOINTS[1].clone(),
            ALL_ENDPOINTS[2].clone(),
        ];
        let shuffled = shuffle_endpoints(sample.clone());
        assert_eq!(shuffled.len(), sample.len());
    }

    #[test]
    fn route_retry_budget_is_one_thousand() {
        assert_eq!(MAX_ROUTE_ATTEMPTS, 1000);
    }

    #[test]
    fn bundled_keys_expose_full_free_pool() {
        let daily = available_endpoints(ModelTier::Daily);
        let reasoning = available_endpoints(ModelTier::Reasoning);
        assert!(!daily.is_empty());
        assert!(!reasoning.is_empty());
    }

    #[test]
    fn provider_failures_failover_across_free_pool() {
        assert!(should_failover_status(400));
        assert!(should_failover_status(422));
        assert!(should_failover_status(429));
        assert!(should_failover_status(503));
    }

    #[test]
    fn route_retry_delay_grows_and_caps() {
        let first = route_retry_delay(1);
        let later = route_retry_delay(40);
        let capped = route_retry_delay(1000);

        assert!(first.as_millis() >= 250);
        assert!(later > first);
        assert!(capped.as_millis() <= 6_255);
    }
}