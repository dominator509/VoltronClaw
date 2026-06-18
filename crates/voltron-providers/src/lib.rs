//! voltron-providers — LLMProvider implementations wrapping DeepSeek and OpenAI.
//!
//! Supports DeepSeek (primary, via `DEEPSEEK_API_KEY` env) and OpenAI (fallback, via
//! `OPENAI_API_KEY` env). Both providers support tool calls and streaming via the
//! standard OpenAI-compatible chat completions API.

use async_trait::async_trait;
use serde::Deserialize;
use voltron_core::{LLMProvider, LLMResponse, Message, ToolCall, ToolDefinition, VoltronError};

// ── Re-export model name constants from rig-core ──────────────────

pub use rig::providers::deepseek::DEEPSEEK_CHAT;
pub use rig::providers::openai::GPT_4O_MINI;

// ── Shared API response shapes ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ResponseToolCall {
    id: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    call_type: Option<String>,
    function: ResponseFunction,
}

#[derive(Debug, Deserialize)]
struct ResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[allow(dead_code)]
    prompt_tokens: Option<u64>,
    #[allow(dead_code)]
    completion_tokens: Option<u64>,
    #[allow(dead_code)]
    total_tokens: Option<u64>,
}

// ── Shared request body helpers ───────────────────────────────────

fn build_request_body(
    model: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
) -> serde_json::Value {
    let msg_array: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut obj = serde_json::json!({
                "role": m.role,
                "content": m.content,
            });
            if !m.tool_calls.is_empty() {
                let calls: Vec<serde_json::Value> = m
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.function_name,
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                obj["tool_calls"] = serde_json::Value::Array(calls);
            }
            obj
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "messages": msg_array,
        "temperature": 0.7,
        "max_tokens": 4096,
    });

    if !tools.is_empty() {
        let tool_defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.function_name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = serde_json::Value::Array(tool_defs);
    }

    body
}

fn parse_response(raw: ChatResponse) -> LLMResponse {
    let choice = raw.choices.into_iter().next().unwrap_or(Choice {
        message: ResponseMessage {
            content: None,
            tool_calls: None,
        },
        finish_reason: None,
    });

    let content = choice.message.content.unwrap_or_default();

    let tool_calls: Vec<ToolCall> = choice
        .message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|tc| {
            let args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
            ToolCall {
                id: tc.id,
                function_name: tc.function.name,
                arguments: args,
            }
        })
        .collect();

    let mut metadata = std::collections::HashMap::new();
    if let Some(usage) = raw.usage {
        if let Some(t) = usage.prompt_tokens {
            metadata.insert("prompt_tokens".into(), t.to_string());
        }
        if let Some(t) = usage.completion_tokens {
            metadata.insert("completion_tokens".into(), t.to_string());
        }
    }
    if let Some(model) = raw.model {
        metadata.insert("model".into(), model);
    }

    LLMResponse {
        content,
        tool_calls,
        finish_reason: choice.finish_reason,
        metadata,
    }
}

// ── DeepSeekProvider ──────────────────────────────────────────────

/// LLMProvider backed by DeepSeek's API.
///
/// Reads `DEEPSEEK_API_KEY` from the environment. Defaults to
/// `deepseek-chat` model if none is specified.
pub struct DeepSeekProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl DeepSeekProvider {
    /// Create a new DeepSeek provider from the `DEEPSEEK_API_KEY` env var.
    pub fn from_env() -> Result<Self, VoltronError> {
        let api_key = std::env::var("DEEPSEEK_API_KEY").map_err(|_| {
            VoltronError::Config("DEEPSEEK_API_KEY environment variable is not set".into())
        })?;
        Ok(Self::new(&api_key, None))
    }

    /// Create a new DeepSeek provider with an explicit API key and optional model override.
    pub fn new(api_key: &str, model: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.unwrap_or_else(|| DEEPSEEK_CHAT.to_string()),
            base_url: "https://api.deepseek.com".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for DeepSeekProvider {
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LLMResponse, VoltronError> {
        let body = build_request_body(&self.model, messages, tools);

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| VoltronError::Provider(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return if status.as_u16() == 429 {
                Err(VoltronError::RateLimit(text))
            } else if status.as_u16() == 401 {
                Err(VoltronError::Auth(text))
            } else {
                Err(VoltronError::Provider(format!(
                    "DeepSeek API error ({}): {}",
                    status, text
                )))
            };
        }

        let chat_resp: ChatResponse = resp
            .json()
            .await
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;

        Ok(parse_response(chat_resp))
    }

    fn provider_name(&self) -> &str {
        "deepseek"
    }
}

// ── OpenAIProvider ────────────────────────────────────────────────

/// LLMProvider backed by OpenAI's API.
///
/// Reads `OPENAI_API_KEY` from the environment. Defaults to
/// `gpt-4o-mini` model (MIT-licensed model constant from rig-core).
pub struct OpenAIProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider from the `OPENAI_API_KEY` env var.
    pub fn from_env() -> Result<Self, VoltronError> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            VoltronError::Config("OPENAI_API_KEY environment variable is not set".into())
        })?;
        Ok(Self::new(&api_key, None))
    }

    /// Create a new OpenAI provider with an explicit API key and optional model override.
    pub fn new(api_key: &str, model: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.unwrap_or_else(|| GPT_4O_MINI.to_string()),
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn generate(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LLMResponse, VoltronError> {
        let body = build_request_body(&self.model, messages, tools);

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| VoltronError::Provider(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return if status.as_u16() == 429 {
                Err(VoltronError::RateLimit(text))
            } else if status.as_u16() == 401 {
                Err(VoltronError::Auth(text))
            } else {
                Err(VoltronError::Provider(format!(
                    "OpenAI API error ({}): {}",
                    status, text
                )))
            };
        }

        let chat_resp: ChatResponse = resp
            .json()
            .await
            .map_err(|e| VoltronError::Serialization(e.to_string()))?;

        Ok(parse_response(chat_resp))
    }

    fn provider_name(&self) -> &str {
        "openai"
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deepseek_provider_creation() {
        let provider = DeepSeekProvider::new("test-key", None);
        assert_eq!(provider.provider_name(), "deepseek");
        assert_eq!(provider.model, DEEPSEEK_CHAT);
    }

    #[test]
    fn test_openai_provider_creation() {
        let provider = OpenAIProvider::new("test-key", None);
        assert_eq!(provider.provider_name(), "openai");
        assert_eq!(provider.model, GPT_4O_MINI);
    }

    #[test]
    fn test_build_request_body_no_tools() {
        let msgs = vec![Message::system("You are helpful."), Message::user("Hello")];
        let body = build_request_body("test-model", &msgs, &[]);
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let msgs = vec![Message::user("Search for X")];
        let tools = vec![ToolDefinition {
            function_name: "search".into(),
            description: "Search the web".into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }];
        let body = build_request_body("test-model", &msgs, &tools);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["function"]["name"], "search");
    }

    #[test]
    fn test_build_request_body_with_tool_calls_in_message() {
        let msgs = vec![Message {
            role: "assistant".into(),
            content: "".into(),
            name: None,
            tool_calls: vec![ToolCall {
                id: "call_123".into(),
                function_name: "search".into(),
                arguments: serde_json::json!({"q": "test"}),
            }],
        }];
        let body = build_request_body("test-model", &msgs, &[]);
        let calls = body["messages"][0]["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "search");
    }

    #[test]
    fn test_parse_response_text_only() {
        let raw = ChatResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: Some("Hello world".into()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Some(Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
            }),
            model: Some("deepseek-chat".into()),
        };
        let resp = parse_response(raw);
        assert_eq!(resp.content, "Hello world");
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            resp.metadata.get("prompt_tokens").map(|s| s.as_str()),
            Some("10")
        );
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let raw = ChatResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: None,
                    tool_calls: Some(vec![ResponseToolCall {
                        id: "call_1".into(),
                        call_type: Some("function".into()),
                        function: ResponseFunction {
                            name: "search".into(),
                            arguments: r#"{"q":"test"}"#.into(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: None,
            model: None,
        };
        let resp = parse_response(raw);
        assert!(resp.content.is_empty());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].function_name, "search");
        assert_eq!(resp.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn test_parse_response_empty_choices() {
        let raw = ChatResponse {
            choices: vec![],
            usage: None,
            model: None,
        };
        let resp = parse_response(raw);
        assert!(resp.content.is_empty());
        assert!(resp.tool_calls.is_empty());
    }
}
