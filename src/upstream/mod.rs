pub mod signer;

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::json;

use crate::error::{classify_upstream, GatewayError};
use crate::openai::{
    ChatCompletionRequest, ChatCompletionResponse, Model, ModelList,
};
use signer::{sha256_hex, sign, sdk_date_now, EMPTY_BODY_SHA256};

/// MaaS / InferHub production chat gateway (used by the official CLI).
pub const CODEARTS_GATEWAY_PROD: &str = "https://snap-access.cn-north-4.myhuaweicloud.com";

const API_PREFIX: &str = "/api/v2";
const PATH_CHAT_COMPLETIONS: &str = "/chat/completions";
const PATH_SESSIONS: &str = "/v1/sessions";
const PATH_PLUGIN_STATS: &str = "/snap-manager/v1/statistics/plugin";

/// Dynamic model list cache TTL.
const MODEL_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// MaaS session reuse window (sessions last ~1h, refresh 10min early).
const SESSION_REUSE: Duration = Duration::from_secs(50 * 60);

/// Tier alias definition: gateway-visible name -> concrete upstream model.
struct Tier {
    name: &'static str,
    model: &'static str,
    effort: &'static str,
}

/// All tiers, in /v1/models display order (mirrors the legacy gateway).
const TIERS: &[Tier] = &[
    Tier { name: "auto", model: "GLM-5.2", effort: "" },
    Tier { name: "fast-model", model: "openpangu-2.0-flash", effort: "" },
    Tier { name: "balanced-model", model: "openpangu-2.0-pro", effort: "" },
    Tier { name: "primary-model", model: "GLM-5.2", effort: "" },
    Tier { name: "deep-model", model: "GLM-5.2", effort: "max" },
];

/// Fallback model list when the dynamic fetch fails (verified 2026-10-10).
const FALLBACK_MODELS: &[&str] = &[
    "GLM-5.2",
    "glm-5.2-sft-harmony",
    "openpangu-2.0-flash",
    "kimi-k2.6-vl",
];

/// Models listed by model_metrics but verified unusable:
/// Qwen3-VL-235B silently returns kimi-k2.6; openpangu-2.0-pro fails with 502.
const BLOCKED_MODELS: &[&str] = &["Qwen3-VL-235B", "openpangu-2.0-pro"];

/// Prompt injected when tools are present so the model emits XML tool calls.
const TOOL_FORMAT_PROMPT: &str = "You are connected to a client that parses tool calls from XML markup.\
When you need to call a tool, output EXACTLY this XML format and nothing else:\n\
<tool name=\"TOOL_NAME\">\n\
<param name=\"PARAM_NAME\">VALUE</param>\n\
</tool>\n\
Do NOT use JSON, do NOT wrap in markdown code blocks, do NOT add extra text.";

/// Resolve a tier alias to (model, effort). Unknown names pass through.
fn resolve_tier(model: &str, effort: &str) -> (String, String) {
    match TIERS.iter().find(|t| t.name == model) {
        Some(tier) => {
            let eff = if effort.is_empty() {
                tier.effort.to_string()
            } else {
                effort.to_string()
            };
            (tier.model.to_string(), eff)
        }
        None => (model.to_string(), effort.to_string()),
    }
}

/// Inject the XML tool-call format prompt into the message list.
fn inject_tool_format_prompt(messages: &[crate::openai::Message]) -> Vec<crate::openai::Message> {
    let mut out = messages.to_vec();
    if let Some(first) = out.first_mut() {
        if first.role == "system" {
            first.content = format!("{}\n\n{}", first.content, TOOL_FORMAT_PROMPT);
            return out;
        }
    }
    out.insert(
        0,
        crate::openai::Message {
            role: "system".to_string(),
            content: TOOL_FORMAT_PROMPT.to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
    );
    out
}

struct ModelCache {
    models: Vec<Model>,
    fetched_at: Option<Instant>,
}

struct MaaSSession {
    id: String,
    created_at: Instant,
}

/// Upstream client for Huawei CodeArts MaaS (AK/SK signed).
pub struct CodeArtsClient {
    ak: String,
    sk: String,
    gateway_url: String,
    base_url: String,
    http: reqwest::Client,
    model_cache: RwLock<ModelCache>,
    session: Mutex<Option<MaaSSession>>,
}

/// Byte stream type returned for SSE proxying.
pub type ByteStream = std::pin::Pin<
    Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>,
>;

impl CodeArtsClient {
    pub fn new(ak: String, sk: String, gateway_url: String) -> Self {
        let gateway = if gateway_url.is_empty() {
            CODEARTS_GATEWAY_PROD.to_string()
        } else {
            gateway_url.trim_end_matches('/').to_string()
        };
        let base_url = if gateway.contains("snap-access") {
            format!("{}{}", CODEARTS_GATEWAY_PROD, API_PREFIX)
        } else if gateway.ends_with(API_PREFIX) {
            gateway.clone()
        } else {
            format!("{}{}", gateway, API_PREFIX)
        };
        // A3: 连接建立超时 10 秒；不设整体超时（SSE 长连接可能持续
        // 数分钟），非流式与流式首帧超时由调用侧分别施加。
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client build");
        Self {
            ak,
            sk,
            gateway_url: gateway,
            base_url,
            http,
            model_cache: RwLock::new(ModelCache {
                models: Vec::new(),
                fetched_at: None,
            }),
            session: Mutex::new(None),
        }
    }

    /// Apply Huawei SDK-HMAC-SHA256 signing headers to a reqwest request.
    fn sign_request(
        &self,
        builder: reqwest::RequestBuilder,
        method: &str,
        url: &url::Url,
        body: Option<&[u8]>,
        extra_headers: &[(&str, &str)],
    ) -> anyhow::Result<reqwest::Request> {
        let x_date = sdk_date_now();
        let host = url
            .host_str()
            .map(|h| match url.port() {
                Some(p) => format!("{}:{}", h, p),
                None => h.to_string(),
            })
            .ok_or_else(|| anyhow::anyhow!("upstream url missing host"))?;

        // Collect all headers that participate in the signature.
        let mut hdrs: Vec<(String, String)> = vec![
            ("host".to_string(), host.clone()),
            ("x-sdk-date".to_string(), x_date.clone()),
        ];
        let payload_hash = match body {
            Some(b) if !b.is_empty() => {
                let hash = sha256_hex(b);
                hdrs.push(("x-sdk-content-sha256".to_string(), hash.clone()));
                hash
            }
            _ => EMPTY_BODY_SHA256.to_string(),
        };
        for (name, value) in extra_headers {
            hdrs.push((name.to_ascii_lowercase(), value.to_string()));
        }
        hdrs.sort_by(|a, b| a.0.cmp(&b.0));
        hdrs.dedup_by(|a, b| a.0 == b.0);

        let sig = sign(
            &self.ak,
            &self.sk,
            method,
            url.path(),
            url.query().unwrap_or(""),
            &hdrs,
            &payload_hash,
            &x_date,
        );

        let mut request = builder
            .header("X-Sdk-Date", &sig.x_sdk_date)
            .header(reqwest::header::HOST, &host)
            .header("Authorization", &sig.authorization);
        if let Some(b) = body {
            if !b.is_empty() {
                request = request.header("X-Sdk-Content-Sha256", sha256_hex(b));
            }
        }
        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }
        Ok(request.build()?)
    }

    /// Ensure a valid MaaS session exists (POST /v1/sessions, reused 50min).
    async fn ensure_session(&self) -> Result<String, GatewayError> {
        // Fast path: reuse under lock.
        {
            let guard = self.session.lock().expect("session lock poisoned");
            if let Some(s) = guard.as_ref() {
                if s.created_at.elapsed() < SESSION_REUSE {
                    return Ok(s.id.clone());
                }
            }
        }

        let url = format!("{}{}", self.gateway_url, PATH_SESSIONS);
        let body = json!({
            "client_type": "CodeArts Agent CLI",
            "device_name": "codearts-gateway",
            "device_type": "Linux",
            "ip_address": "127.0.0.1",
            "client_version": "26.9.3",
        })
        .to_string();

        let parsed = url::Url::parse(&url)
            .map_err(|e| GatewayError::internal(format!("bad session url: {}", e)))?;
        let extra: [(&str, &str); 3] = [
            ("content-type", "application/json"),
            ("agent-type", "PromptCenter"),
            ("x-language", "zh-cn"),
        ];
        let request = self
            .sign_request(
                self.http.post(&url).body(body.clone()),
                "POST",
                &parsed,
                Some(body.as_bytes()),
                &extra,
            )
            .map_err(|e| GatewayError::internal(format!("sign session request: {}", e)))?;

        let resp = self
            .http
            .execute(request)
            .await
            .map_err(|e| classify_upstream(None, &format!("create session: {}", e)))?;
        let status = resp.status();
        let data = resp
            .bytes()
            .await
            .map_err(|e| classify_upstream(None, &format!("read session body: {}", e)))?;
        if !status.is_success() {
            return Err(classify_upstream(
                Some(status.as_u16()),
                &String::from_utf8_lossy(&data[..data.len().min(500)]),
            ));
        }
        let envelope: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|e| GatewayError::bad_gateway(format!("decode session response: {}", e)))?;
        let id = envelope
            .pointer("/data/id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                GatewayError::bad_gateway(format!(
                    "create session: empty session id ({})",
                    envelope.get("message").and_then(|m| m.as_str()).unwrap_or("unknown")
                ))
            })?
            .to_string();

        let mut guard = self.session.lock().expect("session lock poisoned");
        *guard = Some(MaaSSession {
            id: id.clone(),
            created_at: Instant::now(),
        });
        Ok(id)
    }

    /// Build the upstream chat request body (tier resolution + tool prompt).
    fn build_chat_body(&self, req: &ChatCompletionRequest) -> Result<Vec<u8>, GatewayError> {
        req.validate()?;
        let (model, effort) = resolve_tier(&req.model, &req.reasoning_effort);
        let messages = if req.tools.as_ref().map(|t| !t.is_empty()).unwrap_or(false) {
            inject_tool_format_prompt(&req.messages)
        } else {
            req.messages.clone()
        };
        let mut body = json!({
            "model": model,
            "messages": messages,
        });
        if req.stream {
            body["stream"] = json!(true);
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            body["max_tokens"] = json!(m);
        }
        if let Some(tools) = &req.tools {
            body["tools"] = serde_json::to_value(tools).unwrap_or(json!([]));
        }
        if let Some(tc) = &req.tool_choice {
            body["tool_choice"] = serde_json::to_value(tc).unwrap_or(serde_json::Value::Null);
        }
        if !effort.is_empty() {
            body["reasoning_effort"] = json!(effort);
        }
        serde_json::to_vec(&body)
            .map_err(|e| GatewayError::internal(format!("encode chat body: {}", e)))
    }

    /// Non-streaming chat completion with a 120s overall timeout (A3).
    pub async fn chat_completion(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, GatewayError> {
        let body = self.build_chat_body(req)?;
        let data = tokio::time::timeout(
            Duration::from_secs(120),
            self.post_signed(PATH_CHAT_COMPLETIONS, &body),
        )
        .await
        .map_err(|_| {
            GatewayError::bad_gateway("upstream timeout after 120s")
        })??;
        parse_chat_response(&data, &req.model)
    }

    /// Streaming chat completion: returns the upstream SSE byte stream as-is.
    pub async fn chat_completion_stream(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<ByteStream, GatewayError> {
        let body = self.build_chat_body(req)?;
        let session_id = self.ensure_session().await?;
        let url = format!("{}{}", self.base_url, PATH_CHAT_COMPLETIONS);
        let parsed = url::Url::parse(&url)
            .map_err(|e| GatewayError::internal(format!("bad chat url: {}", e)))?;
        let session = session_id.clone();
        let extra: [(&str, &str); 4] = [
            ("content-type", "application/json"),
            ("accept", "text/event-stream"),
            ("x-ot-session-id", &session),
            ("user-session-id", &session),
        ];
        let request = self
            .sign_request(
                self.http.post(&url).body(body.clone()),
                "POST",
                &parsed,
                Some(&body),
                &extra,
            )
            .map_err(|e| GatewayError::internal(format!("sign stream request: {}", e)))?;
        let resp = self
            .http
            .execute(request)
            .await
            .map_err(|e| classify_upstream(None, &e.to_string()))?;
        let status = resp.status();
        if status.as_u16() >= 400 {
            let data = resp.bytes().await.unwrap_or_default();
            return Err(classify_upstream(
                Some(status.as_u16()),
                &String::from_utf8_lossy(&data[..data.len().min(500)]),
            ));
        }
        // A3: 流式首字节超时——收到响应头到第一个 SSE 数据帧最多 30 秒；
        // 首帧到达后不再受该超时约束（长回答可能持续数分钟）。
        let mut stream = resp.bytes_stream();
        let first = tokio::time::timeout(Duration::from_secs(30), stream.next())
            .await
            .map_err(|_| GatewayError::bad_gateway("upstream timeout after 30s"))?
            .transpose()
            .map_err(|e| classify_upstream(None, &e.to_string()))?;
        let first_stream = match first {
            Some(chunk) => futures_util::stream::iter(vec![Ok(chunk)]).chain(stream),
            None => stream,
        };
        Ok(Box::pin(first_stream))
    }

    /// Signed POST to a chat-API path with the MaaS session headers.
    async fn post_signed(
        &self,
        path: &str,
        body: &[u8],
    ) -> Result<Bytes, GatewayError> {
        let session_id = self.ensure_session().await?;
        let url = format!("{}{}", self.base_url, path);
        let parsed = url::Url::parse(&url)
            .map_err(|e| GatewayError::internal(format!("bad url: {}", e)))?;
        let session = session_id.clone();
        let extra: [(&str, &str); 3] = [
            ("content-type", "application/json"),
            ("x-ot-session-id", &session),
            ("user-session-id", &session),
        ];
        let request = self
            .sign_request(
                self.http.post(&url).body(body.to_vec()),
                "POST",
                &parsed,
                Some(body),
                &extra,
            )
            .map_err(|e| GatewayError::internal(format!("sign request: {}", e)))?;
        let resp = self
            .http
            .execute(request)
            .await
            .map_err(|e| classify_upstream(None, &e.to_string()))?;
        let status = resp.status();
        let data = resp
            .bytes()
            .await
            .map_err(|e| classify_upstream(None, &e.to_string()))?;
        if !status.is_success() {
            return Err(classify_upstream(
                Some(status.as_u16()),
                &String::from_utf8_lossy(&data[..data.len().min(500)]),
            ));
        }
        Ok(data)
    }

    /// Dynamic model list: tier aliases + account-available models.
    pub async fn models(&self) -> Result<ModelList, GatewayError> {
        let upstream_models = self.upstream_models().await;
        let mut data: Vec<Model> = TIERS
            .iter()
            .map(|t| Model {
                id: t.name.to_string(),
                object: "model".to_string(),
                owned_by: "codearts-gateway".to_string(),
            })
            .collect();
        data.extend(upstream_models);
        Ok(ModelList {
            object: "list".to_string(),
            data,
        })
    }

    /// Fetch available models from /snap-manager/v1/statistics/plugin with
    /// a 10-minute cache; falls back to the static list on any failure.
    async fn upstream_models(&self) -> Vec<Model> {
        {
            let cache = self.model_cache.read().expect("model cache poisoned");
            if let Some(at) = cache.fetched_at {
                if at.elapsed() < MODEL_CACHE_TTL && !cache.models.is_empty() {
                    return cache.models.clone();
                }
            }
        }

        let fallback = || {
            FALLBACK_MODELS
                .iter()
                .map(|id| Model {
                    id: id.to_string(),
                    object: "model".to_string(),
                    owned_by: "huaweicloud-maas".to_string(),
                })
                .collect::<Vec<_>>()
        };

        let url = format!("{}{}", self.gateway_url, PATH_PLUGIN_STATS);
        let Ok(parsed) = url::Url::parse(&url) else {
            return fallback();
        };
        let extra: [(&str, &str); 3] = [
            ("content-type", "application/json"),
            ("agent-type", "PromptCenter"),
            ("x-language", "zh-cn"),
        ];
        let Ok(request) =
            self.sign_request(self.http.get(&url), "GET", &parsed, None, &extra)
        else {
            return fallback();
        };
        let Ok(resp) = self.http.execute(request).await else {
            return fallback();
        };
        if !resp.status().is_success() {
            return fallback();
        }
        let Ok(data) = resp.bytes().await else {
            return fallback();
        };
        let Ok(envelope) = serde_json::from_slice::<serde_json::Value>(&data) else {
            return fallback();
        };
        let Some(metrics) = envelope.get("model_metrics").and_then(|m| m.as_array()) else {
            return fallback();
        };
        let mut out: Vec<Model> = metrics
            .iter()
            .filter(|m| {
                m.get("enable").and_then(|v| v.as_bool()).unwrap_or(false)
                    && m.get("status").and_then(|v| v.as_str()) == Some("normal")
            })
            .filter_map(|m| m.get("model_id").and_then(|v| v.as_str()))
            .filter(|id| !id.is_empty() && !BLOCKED_MODELS.contains(id))
            .map(|id| Model {
                id: id.to_string(),
                object: "model".to_string(),
                owned_by: "huaweicloud-maas".to_string(),
            })
            .collect();
        if out.is_empty() {
            return fallback();
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        let mut cache = self.model_cache.write().expect("model cache poisoned");
        cache.models = out.clone();
        cache.fetched_at = Some(Instant::now());
        out
    }
}

/// Parse a possibly-SSE upstream body into a complete chat response:
/// 1) direct JSON; 2) strip a leading "data:" prefix; 3) aggregate SSE chunks.
pub fn parse_chat_response(
    data: &[u8],
    default_model: &str,
) -> Result<ChatCompletionResponse, GatewayError> {
    if let Ok(resp) = serde_json::from_slice::<ChatCompletionResponse>(data) {
        if !resp.choices.is_empty() {
            return Ok(normalize_chat_response(resp, default_model));
        }
    }
    let stripped = strip_data_prefix(data);
    if !stripped.is_empty() && stripped.as_slice() != data {
        if let Ok(resp) = serde_json::from_slice::<ChatCompletionResponse>(&stripped) {
            if !resp.choices.is_empty() {
                return Ok(normalize_chat_response(resp, default_model));
            }
        }
    }
    parse_sse_chat(data)
        .map(|resp| normalize_chat_response(resp, default_model))
        .ok_or_else(|| GatewayError::bad_gateway("no chat completion in upstream response"))
}

fn strip_data_prefix(data: &[u8]) -> Vec<u8> {
    let s = String::from_utf8_lossy(data);
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("data:") {
        rest.trim().as_bytes().to_vec()
    } else {
        data.to_vec()
    }
}

/// Aggregate chat.completion.chunk frames from an SSE byte stream.
fn parse_sse_chat(data: &[u8]) -> Option<ChatCompletionResponse> {
    let text = String::from_utf8_lossy(data);
    let mut id = String::new();
    let mut model = String::new();
    let mut created: i64 = 0;
    let mut role = "assistant".to_string();
    let mut finish = "stop".to_string();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut usage: Option<crate::openai::Usage> = None;

    for line in text.lines() {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(chunk) = serde_json::from_str::<crate::openai::ChatCompletionChunk>(payload) else {
            continue;
        };
        if id.is_empty() {
            id = chunk.id;
        }
        if model.is_empty() {
            model = chunk.model;
        }
        if created == 0 {
            created = chunk.created;
        }
        for choice in &chunk.choices {
            if !choice.delta.role.is_empty() {
                role = choice.delta.role.clone();
            }
            content.push_str(&choice.delta.content);
            reasoning.push_str(&choice.delta.reasoning_content);
            if !choice.finish_reason.is_empty() {
                finish = choice.finish_reason.clone();
            }
        }
        if let Some(u) = chunk.usage {
            if u.total_tokens > 0 {
                usage = Some(u);
            }
        }
    }

    if content.is_empty() && reasoning.is_empty() {
        return None;
    }
    let mut full = reasoning;
    if !full.is_empty() && !content.is_empty() {
        full.push_str("\n\n");
    }
    full.push_str(&content);

    Some(ChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created,
        model,
        choices: vec![crate::openai::Choice {
            index: 0,
            message: crate::openai::Message {
                role,
                content: full,
                tool_calls: None,
                tool_call_id: None,
            },
            finish_reason: finish,
        }],
        usage,
    })
}

/// Fill defaults for missing role/finish_reason/model fields.
fn normalize_chat_response(
    mut resp: ChatCompletionResponse,
    default_model: &str,
) -> ChatCompletionResponse {
    if resp.model.is_empty() {
        resp.model = default_model.to_string();
    }
    if resp.object.is_empty() {
        resp.object = "chat.completion".to_string();
    }
    for choice in &mut resp.choices {
        if choice.message.role.is_empty() {
            choice.message.role = "assistant".to_string();
        }
        if choice.finish_reason.is_empty() {
            choice.finish_reason = "stop".to_string();
        }
    }
    resp
}

/// Type alias so main.rs can refer to the client without generics noise.
pub type SharedClient = Arc<CodeArtsClient>;
