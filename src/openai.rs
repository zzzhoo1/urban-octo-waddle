use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;

/// Chat message. `content` accepts both a plain string and the multimodal
/// array form ([{"type":"text","text":"..."},...]); deserialization
/// always normalizes to a plain string so the upstream stays compatible.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Box<RawValue>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            role: String,
            content: Option<serde_json::Value>,
            tool_calls: Option<Vec<Box<RawValue>>>,
            tool_call_id: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let content = match raw.content {
            None | Some(serde_json::Value::Null) => String::new(),
            Some(serde_json::Value::String(s)) => s,
            Some(serde_json::Value::Array(parts)) => {
                let mut out = String::new();
                for part in &parts {
                    if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                if !out.is_empty() {
                                    out.push('\n');
                                }
                                out.push_str(text);
                            }
                        }
                    }
                }
                out
            }
            Some(other) => other.to_string(),
        };
        Ok(Message {
            role: raw.role,
            content,
            tool_calls: raw.tool_calls,
            tool_call_id: raw.tool_call_id,
        })
    }
}

/// OpenAI-compatible chat completion request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Box<RawValue>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Box<RawValue>>,
    /// Reasoning effort tier (low/medium/high/xhigh/max), passed to upstream.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning_effort: String,
    /// B2 gateway extension: mark this request as a background task.
    /// Never forwarded upstream (build_chat_body only picks known fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
}

impl ChatCompletionRequest {
    pub fn validate(&self) -> Result<(), crate::error::GatewayError> {
        if self.model.is_empty() {
            return Err(crate::error::GatewayError::bad_request("model is required"));
        }
        if self.messages.is_empty() {
            return Err(crate::error::GatewayError::bad_request(
                "messages must not be empty",
            ));
        }
        Ok(())
    }
}

/// Nested cache detail reported by some providers (OpenAI style).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PromptTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    /// Actual credit deducted by the upstream. None = unknown; the panel
    /// shows "—" in that case, which is deliberately distinct from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<f64>,
    /// Flat cache-hit counter used by some providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_hit_tokens: Option<u64>,
    /// OpenAI-style nested cache detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
}

impl Usage {
    /// Cached tokens from any of the known response shapes.
    pub fn cached_tokens(&self) -> Option<u64> {
        self.prompt_cache_hit_tokens.or_else(|| {
            self.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens)
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Choice {
    #[serde(default)]
    pub index: u32,
    pub message: Message,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finish_reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_completion_object")]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

fn default_completion_object() -> String {
    "chat.completion".to_string()
}

/// Streaming chunk types (SSE data frames).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChunkDelta {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reasoning_content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkChoice {
    #[serde(default)]
    pub delta: ChunkDelta,
    #[serde(default)]
    pub finish_reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionChunk {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub owned_by: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelList {
    pub object: String,
    pub data: Vec<Model>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}
