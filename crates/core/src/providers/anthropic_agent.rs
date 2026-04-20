//! Anthropic Managed Agents provider — runs Claude as a remote agent with
//! server-side tool execution via the Claude Agent SDK.
//!
//! Key differences from the regular Anthropic provider:
//! - Tools execute **server-side** in a cloud container (Bash, file ops, etc.)
//! - Sessions persist context server-side across turns (no local history needed)
//! - Uses `previous_session_id` to chain sessions
//! - Events streamed via SSE from `GET /v1/sessions/{id}/stream`
//! - User messages sent via `POST /v1/sessions/{id}/events`
//!
//! Auth: standard `ANTHROPIC_API_KEY` + `anthropic-beta: managed-agents-2026-04-01`.
//!
//! Model prefix: `agent/` (e.g. `agent/claude-sonnet-4-5`).

use super::{EventStream, ModelInfo, Provider, ProviderEvent, StreamRequest};
use crate::error::{Error, Result};
use async_stream::try_stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Mutex;

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const BETA_HEADER: &str = "managed-agents-2026-04-01";

pub struct AnthropicAgentProvider {
    client: Client,
    api_key: String,
    base_url: String,
    /// Reusable session ID — persists context across turns.
    session_id: Mutex<Option<String>>,
    /// Agent ID — created once on first use.
    agent_id: Mutex<Option<String>>,
}

impl AnthropicAgentProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            session_id: Mutex::new(None),
            agent_id: Mutex::new(None),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn headers(&self) -> Vec<(&str, String)> {
        vec![
            ("x-api-key", self.api_key.clone()),
            ("anthropic-version", API_VERSION.to_string()),
            ("anthropic-beta", BETA_HEADER.to_string()),
            ("content-type", "application/json".to_string()),
        ]
    }

    fn apply_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut r = req;
        for (k, v) in self.headers() {
            r = r.header(k, v);
        }
        r
    }

    /// Create a managed agent (once per provider lifetime).
    async fn ensure_agent(&self, model: &str) -> Result<String> {
        {
            let guard = self.agent_id.lock().unwrap();
            if let Some(id) = guard.as_ref() {
                return Ok(id.clone());
            }
        }

        // Strip "managed/" prefix for the actual model name.
        let model_name = model.strip_prefix("managed/").unwrap_or(model);

        // `/v1/agents` requires `name` and rejects `max_tokens`
        // ("Extra inputs are not permitted"). The name is a descriptive label
        // stored server-side; we use a static one per provider instance.
        let body = json!({
            "name": "thclaws",
            "model": model_name,
            "tools": [{"type": "agent_toolset_20260401"}],
        });

        let resp = self
            .apply_headers(self.client.post(format!("{}/v1/agents", self.base_url)))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("create agent: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("create agent failed: {text}")));
        }

        let v: Value = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("agent json: {e}")))?;
        let id = v["id"]
            .as_str()
            .ok_or_else(|| Error::Provider("agent response missing id".into()))?
            .to_string();

        *self.agent_id.lock().unwrap() = Some(id.clone());
        Ok(id)
    }

    /// Create or reuse a session.
    async fn ensure_session(&self, agent_id: &str) -> Result<String> {
        {
            let guard = self.session_id.lock().unwrap();
            if let Some(id) = guard.as_ref() {
                return Ok(id.clone());
            }
        }

        let body = json!({
            "agent": agent_id,
        });

        let resp = self
            .apply_headers(self.client.post(format!("{}/v1/sessions", self.base_url)))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("create session: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("create session failed: {text}")));
        }

        let v: Value = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("session json: {e}")))?;
        let id = v["id"]
            .as_str()
            .ok_or_else(|| Error::Provider("session response missing id".into()))?
            .to_string();

        *self.session_id.lock().unwrap() = Some(id.clone());
        Ok(id)
    }

    /// Send user message to the session.
    async fn send_message(&self, session_id: &str, text: &str) -> Result<()> {
        let body = json!({
            "events": [{
                "type": "user.message",
                "content": [{"type": "text", "text": text}]
            }]
        });

        let resp = self
            .apply_headers(self.client.post(format!(
                "{}/v1/sessions/{}/events",
                self.base_url, session_id
            )))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("send event: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("send event failed: {text}")));
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for AnthropicAgentProvider {
    async fn stream(&self, req: StreamRequest) -> Result<EventStream> {
        let agent_id = self.ensure_agent(&req.model).await?;
        let session_id = self.ensure_session(&agent_id).await?;

        // Extract user message text from the last message.
        let user_text = req
            .messages
            .last()
            .and_then(|m| {
                m.content.iter().find_map(|b| match b {
                    crate::types::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .unwrap_or_default();

        // Open SSE stream first, then send the message.
        let stream_resp = self
            .apply_headers(self.client.get(format!(
                "{}/v1/sessions/{}/stream",
                self.base_url, session_id
            )))
            .send()
            .await
            .map_err(|e| Error::Provider(format!("stream connect: {e}")))?;

        if !stream_resp.status().is_success() {
            let text = stream_resp.text().await.unwrap_or_default();
            return Err(Error::Provider(format!("stream failed: {text}")));
        }

        // Send the user message.
        self.send_message(&session_id, &user_text).await?;

        let byte_stream = stream_resp.bytes_stream();
        let model = req.model.clone();
        let raw_dump = super::RawDump::new(format!("anthropic-agent {}", model));

        let event_stream = try_stream! {
            let mut buffer = String::new();
            let mut byte_stream = Box::pin(byte_stream);
            let mut seen_start = false;
            let mut raw = raw_dump;

            while let Some(chunk) = byte_stream.next().await {
                let chunk = chunk.map_err(|e| Error::Provider(format!("stream read: {e}")))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(boundary) = buffer.find("\n\n") {
                    let event_text: String = buffer.drain(..boundary + 2).collect();
                    let trimmed = event_text.trim();

                    // Parse SSE: find "data:" line
                    let mut data_line = None;
                    for line in trimmed.lines() {
                        if let Some(rest) = line.strip_prefix("data: ") {
                            data_line = Some(rest);
                        }
                    }
                    let Some(data) = data_line else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(data) else { continue };

                    let event_type = v.get("type").and_then(Value::as_str).unwrap_or("");

                    if !seen_start {
                        yield ProviderEvent::MessageStart { model: model.clone() };
                        seen_start = true;
                    }

                    match event_type {
                        "agent.message" => {
                            // Extract text content from the message.
                            if let Some(content) = v.get("content").and_then(Value::as_array) {
                                for block in content {
                                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                                        if !text.is_empty() {
                                            raw.push(text);
                                            yield ProviderEvent::TextDelta(text.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        "agent.tool_use" => {
                            // Server-side tool execution — display as info, don't trigger local execution.
                            let name = v.get("name").and_then(Value::as_str).unwrap_or("unknown");
                            yield ProviderEvent::TextDelta(
                                format!("\n🔧 [server tool: {name}]\n")
                            );
                        }
                        "agent.tool_result" => {
                            // Tool finished server-side.
                            let output = v.pointer("/content/0/text")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            if !output.is_empty() {
                                let preview: String = output.chars().take(200).collect();
                                yield ProviderEvent::TextDelta(
                                    format!("  → {preview}\n")
                                );
                            }
                        }
                        "session.status_idle" | "session.paused" => {
                            // Agent is done — end the turn.
                            raw.flush();
                            yield ProviderEvent::MessageStop {
                                stop_reason: Some("end_turn".into()),
                                usage: None,
                            };
                            return;
                        }
                        "error" => {
                            let msg = v.get("message").and_then(Value::as_str).unwrap_or("unknown error");
                            yield ProviderEvent::TextDelta(format!("\n⚠ Error: {msg}\n"));
                            yield ProviderEvent::MessageStop {
                                stop_reason: Some("error".into()),
                                usage: None,
                            };
                            return;
                        }
                        _ => {
                            // Ignore unknown events.
                        }
                    }
                }
            }

            // Stream ended without idle — emit stop anyway.
            raw.flush();
            if seen_start {
                yield ProviderEvent::MessageStop {
                    stop_reason: Some("end_turn".into()),
                    usage: None,
                };
            }
        };

        Ok(Box::pin(event_stream))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // Fetch from Anthropic API and prefix with "managed/".
        let anthropic = crate::providers::anthropic::AnthropicProvider::new(&self.api_key);
        let mut models = anthropic.list_models().await?;
        for m in &mut models {
            m.id = format!("managed/{}", m.id);
            if let Some(ref name) = m.display_name {
                m.display_name = Some(format!("{} (Managed Agent)", name));
            }
        }
        Ok(models)
    }
}
