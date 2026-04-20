//! Stream adapter: fold raw [`ProviderEvent`]s into semantic [`AssembledEvent`]s.
//!
//! - `TextDelta` passes through as `AssembledEvent::Text` (for streaming UI).
//! - `ToolUseStart` + N× `ToolUseDelta` + `ContentBlockStop` collapses into a
//!   single `AssembledEvent::ToolUse` with a fully-parsed JSON input.
//! - `MessageStop` becomes `AssembledEvent::Done`.
//!
//! The agent loop typically drains this via [`collect_turn`] to get a complete
//! turn result, or consumes it live when the UI wants streaming text.

use crate::error::{Error, Result};
use crate::providers::{ProviderEvent, Usage};
use crate::types::ContentBlock;
use async_stream::try_stream;
use futures::{Stream, StreamExt};

#[derive(Debug, Clone, PartialEq)]
pub enum AssembledEvent {
    Text(String),
    /// Always `ContentBlock::ToolUse { id, name, input }`.
    ToolUse(ContentBlock),
    Done {
        stop_reason: Option<String>,
        usage: Option<Usage>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct TurnResult {
    pub text: String,
    /// Each entry is a `ContentBlock::ToolUse`.
    pub tool_uses: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

enum BlockState {
    None,
    Text,
    ToolUse {
        id: String,
        name: String,
        buf: String,
    },
}

pub fn assemble<S>(inner: S) -> impl Stream<Item = Result<AssembledEvent>> + Send + 'static
where
    S: Stream<Item = Result<ProviderEvent>> + Send + 'static,
{
    try_stream! {
        let mut state = BlockState::None;
        let mut inner = Box::pin(inner);
        while let Some(ev) = inner.next().await {
            let ev = ev?;
            match ev {
                ProviderEvent::MessageStart { .. } => {}
                ProviderEvent::TextDelta(s) => {
                    state = BlockState::Text;
                    yield AssembledEvent::Text(s);
                }
                ProviderEvent::ToolUseStart { id, name } => {
                    state = BlockState::ToolUse {
                        id,
                        name,
                        buf: String::new(),
                    };
                }
                ProviderEvent::ToolUseDelta { partial_json } => {
                    if let BlockState::ToolUse { buf, .. } = &mut state {
                        buf.push_str(&partial_json);
                    }
                }
                ProviderEvent::ContentBlockStop => {
                    let prev = std::mem::replace(&mut state, BlockState::None);
                    if let BlockState::ToolUse { id, name, buf } = prev {
                        let input: serde_json::Value = if buf.trim().is_empty() {
                            serde_json::json!({})
                        } else {
                            serde_json::from_str(&buf).map_err(|e| {
                                Error::Provider(format!(
                                    "tool_use json parse: {e} (buf={buf})"
                                ))
                            })?
                        };
                        yield AssembledEvent::ToolUse(ContentBlock::ToolUse { id, name, input });
                    }
                }
                ProviderEvent::MessageStop { stop_reason, usage } => {
                    yield AssembledEvent::Done { stop_reason, usage };
                }
            }
        }
    }
}

pub async fn collect_turn<S>(stream: S) -> Result<TurnResult>
where
    S: Stream<Item = Result<AssembledEvent>> + Send,
{
    let mut out = TurnResult::default();
    let mut stream = Box::pin(stream);
    while let Some(ev) = stream.next().await {
        match ev? {
            AssembledEvent::Text(s) => out.text.push_str(&s),
            AssembledEvent::ToolUse(block) => out.tool_uses.push(block),
            AssembledEvent::Done { stop_reason, usage } => {
                out.stop_reason = stop_reason;
                out.usage = usage;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    fn src(events: Vec<ProviderEvent>) -> impl Stream<Item = Result<ProviderEvent>> + Send {
        stream::iter(events.into_iter().map(Ok))
    }

    async fn collected(events: Vec<ProviderEvent>) -> TurnResult {
        collect_turn(assemble(src(events))).await.unwrap()
    }

    #[tokio::test]
    async fn text_only_turn() {
        let r = collected(vec![
            ProviderEvent::MessageStart { model: "m".into() },
            ProviderEvent::TextDelta("Hello".into()),
            ProviderEvent::TextDelta(", world".into()),
            ProviderEvent::ContentBlockStop,
            ProviderEvent::MessageStop {
                stop_reason: Some("end_turn".into()),
                usage: Some(Usage {
                    input_tokens: 3,
                    output_tokens: 2,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                }),
            },
        ])
        .await;

        assert_eq!(r.text, "Hello, world");
        assert_eq!(r.tool_uses.len(), 0);
        assert_eq!(r.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(r.usage.unwrap().output_tokens, 2);
    }

    #[tokio::test]
    async fn tool_use_accumulates_partial_json() {
        let r = collected(vec![
            ProviderEvent::ToolUseStart {
                id: "toolu_1".into(),
                name: "read_file".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "{\"pa".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "th\":\"".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "/tmp/x\"}".into(),
            },
            ProviderEvent::ContentBlockStop,
            ProviderEvent::MessageStop {
                stop_reason: Some("tool_use".into()),
                usage: None,
            },
        ])
        .await;

        assert_eq!(r.text, "");
        assert_eq!(r.tool_uses.len(), 1);
        match &r.tool_uses[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "read_file");
                assert_eq!(input, &serde_json::json!({"path": "/tmp/x"}));
            }
            _ => panic!("expected ToolUse"),
        }
        assert_eq!(r.stop_reason.as_deref(), Some("tool_use"));
    }

    #[tokio::test]
    async fn text_then_tool_use_in_one_turn() {
        let r = collected(vec![
            ProviderEvent::TextDelta("Let me check ".into()),
            ProviderEvent::TextDelta("that file.".into()),
            ProviderEvent::ContentBlockStop,
            ProviderEvent::ToolUseStart {
                id: "toolu_2".into(),
                name: "glob".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "{\"pattern\":\"*.rs\"}".into(),
            },
            ProviderEvent::ContentBlockStop,
            ProviderEvent::MessageStop {
                stop_reason: Some("tool_use".into()),
                usage: None,
            },
        ])
        .await;

        assert_eq!(r.text, "Let me check that file.");
        assert_eq!(r.tool_uses.len(), 1);
        if let ContentBlock::ToolUse { name, input, .. } = &r.tool_uses[0] {
            assert_eq!(name, "glob");
            assert_eq!(input["pattern"], "*.rs");
        } else {
            panic!("expected ToolUse");
        }
    }

    #[tokio::test]
    async fn two_tool_uses_in_one_turn() {
        let r = collected(vec![
            ProviderEvent::ToolUseStart {
                id: "a".into(),
                name: "read".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "{\"p\":1}".into(),
            },
            ProviderEvent::ContentBlockStop,
            ProviderEvent::ToolUseStart {
                id: "b".into(),
                name: "write".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "{\"p\":2}".into(),
            },
            ProviderEvent::ContentBlockStop,
            ProviderEvent::MessageStop {
                stop_reason: Some("tool_use".into()),
                usage: None,
            },
        ])
        .await;

        assert_eq!(r.tool_uses.len(), 2);
        let ids: Vec<_> = r
            .tool_uses
            .iter()
            .map(|b| match b {
                ContentBlock::ToolUse { id, .. } => id.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn empty_tool_input_becomes_empty_object() {
        let r = collected(vec![
            ProviderEvent::ToolUseStart {
                id: "x".into(),
                name: "list_projects".into(),
            },
            ProviderEvent::ContentBlockStop,
            ProviderEvent::MessageStop {
                stop_reason: Some("tool_use".into()),
                usage: None,
            },
        ])
        .await;
        assert_eq!(r.tool_uses.len(), 1);
        if let ContentBlock::ToolUse { input, .. } = &r.tool_uses[0] {
            assert_eq!(input, &serde_json::json!({}));
        } else {
            panic!("expected ToolUse");
        }
    }

    #[tokio::test]
    async fn malformed_tool_json_yields_error() {
        let result = collect_turn(assemble(src(vec![
            ProviderEvent::ToolUseStart {
                id: "x".into(),
                name: "t".into(),
            },
            ProviderEvent::ToolUseDelta {
                partial_json: "{not-json".into(),
            },
            ProviderEvent::ContentBlockStop,
        ])))
        .await;
        assert!(result.is_err(), "expected parse error");
        assert!(format!("{:?}", result.unwrap_err()).contains("tool_use json parse"));
    }
}
