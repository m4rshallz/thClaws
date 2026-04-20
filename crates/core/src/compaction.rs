//! Message-window compaction with summarization support.
//!
//! Two strategies:
//! 1. **`compact()`** — fast, synchronous drop-oldest. Used as a fallback.
//! 2. **`compact_with_summary()`** — async, uses the provider to summarize
//!    older messages into a compact text block before dropping. Preserves
//!    more context quality at the cost of an extra API call.
//!
//! The agent loop calls `compact_with_summary()` when a provider is
//! available, falling back to `compact()` on error.

use crate::tokens::estimate_tokens;
use crate::types::{ContentBlock, Message, Role};

/// Approximate the token cost of a single message.
pub fn estimate_message_tokens(m: &Message) -> usize {
    let mut chunks: Vec<String> = Vec::new();
    for block in &m.content {
        match block {
            ContentBlock::Text { text } => chunks.push(text.clone()),
            ContentBlock::ToolUse { name, input, .. } => {
                chunks.push(name.clone());
                chunks.push(input.to_string());
            }
            ContentBlock::ToolResult { content, .. } => chunks.push(content.clone()),
        }
    }
    estimate_tokens(&chunks.join(" "))
}

/// Sum message tokens across a slice.
pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Fast synchronous compaction: drop oldest messages until under budget.
/// Preserves tool_use/tool_result pairs — never splits a tool call from its
/// result, as that would confuse the provider.
pub fn compact(messages: &[Message], budget_tokens: usize) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }
    let mut start = 0;
    while start < messages.len().saturating_sub(1)
        && estimate_messages_tokens(&messages[start..]) > budget_tokens
    {
        start += 1;
        // Don't split tool_use from its tool_result: if message at `start`
        // contains a ToolResult, skip it too (drop the orphaned result).
        if start < messages.len() {
            let has_tool_result = messages[start]
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }));
            if has_tool_result {
                start += 1;
            }
        }
    }
    messages[start..].to_vec()
}

/// Summarize older messages into a compact text block, then keep only
/// the summary + recent messages. Returns the compacted message list.
///
/// Strategy:
/// 1. Split messages into "old" (to be summarized) and "recent" (to keep).
///    Keep at least the last 4 messages (2 turns) untouched.
/// 2. Render old messages into a text block for the summarizer.
/// 3. Call the provider to generate a summary (max 2K tokens output).
/// 4. Prepend a synthetic user message with the summary, then append recent.
/// 5. If the API call fails, fall back to drop-oldest.
pub async fn compact_with_summary(
    messages: &[Message],
    budget_tokens: usize,
    provider: &dyn crate::providers::Provider,
    model: &str,
) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }

    let total = estimate_messages_tokens(messages);
    if total <= budget_tokens {
        return messages.to_vec();
    }

    // Keep at least the last 4 messages (2 user-assistant turns).
    let keep_recent = messages.len().min(4).max(1);
    let split_at = messages.len().saturating_sub(keep_recent);
    if split_at == 0 {
        return compact(messages, budget_tokens);
    }

    let old = &messages[..split_at];
    let recent = &messages[split_at..];

    // Render old messages into a summarizable text.
    let rendered = render_for_summary(old);
    if rendered.is_empty() {
        return compact(messages, budget_tokens);
    }

    // Ask the provider to summarize.
    let summary_prompt = crate::prompts::render_named(
        "compaction",
        crate::prompts::defaults::COMPACTION,
        &[("conversation", &rendered)],
    );
    let summary_system = crate::prompts::load(
        "compaction_system",
        crate::prompts::defaults::COMPACTION_SYSTEM,
    );

    let req = crate::providers::StreamRequest {
        model: model.to_string(),
        system: Some(summary_system),
        messages: vec![Message::user(summary_prompt)],
        tools: vec![],
        max_tokens: 2048,
        thinking_budget: None,
    };

    match provider.stream(req).await {
        Ok(stream) => {
            let result = crate::providers::collect_turn(crate::providers::assemble(stream)).await;
            match result {
                Ok(turn) if !turn.text.is_empty() => {
                    let mut out = Vec::with_capacity(1 + recent.len());
                    // Synthetic summary message as a system-context user message.
                    out.push(Message {
                        role: Role::User,
                        content: vec![ContentBlock::Text {
                            text: format!(
                                "[Conversation summary — earlier messages were compacted]\n\n{}",
                                turn.text
                            ),
                        }],
                    });
                    out.extend_from_slice(recent);
                    out
                }
                _ => compact(messages, budget_tokens),
            }
        }
        Err(_) => compact(messages, budget_tokens),
    }
}

/// Render messages into a human-readable text for summarization.
fn render_for_summary(messages: &[Message]) -> String {
    let mut lines = Vec::new();
    for m in messages {
        let role = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
        };
        let text: String = m
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                ContentBlock::ToolUse { name, input, .. } => {
                    Some(format!("[Called tool: {name} with {}]", input))
                }
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let prefix = if *is_error { "Error" } else { "Result" };
                    let preview: String = content.chars().take(500).collect();
                    Some(format!("[{prefix}: {preview}]"))
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            lines.push(format!("{role}: {text}"));
        }
    }
    lines.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;

    fn text_msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    #[test]
    fn estimate_message_tokens_counts_text_block() {
        let m = text_msg(Role::User, &"a".repeat(28));
        assert_eq!(estimate_message_tokens(&m), 10);
    }

    #[test]
    fn estimate_message_tokens_sums_blocks() {
        let m = Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "aaaaa".into(),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "id".into(),
                    content: "bbbbb".into(),
                    is_error: false,
                },
            ],
        };
        assert_eq!(estimate_message_tokens(&m), 4);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(compact(&[], 100).is_empty());
    }

    #[test]
    fn under_budget_is_unchanged() {
        let msgs = vec![
            text_msg(Role::User, "hi"),
            text_msg(Role::Assistant, "hello"),
        ];
        let out = compact(&msgs, 10_000);
        assert_eq!(out, msgs);
    }

    #[test]
    fn over_budget_drops_oldest_first() {
        let s = "a".repeat(28);
        let msgs = vec![
            text_msg(Role::User, &s),
            text_msg(Role::Assistant, &s),
            text_msg(Role::User, &s),
            text_msg(Role::Assistant, &s),
        ];
        let out = compact(&msgs, 25);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], msgs[2]);
        assert_eq!(out[1], msgs[3]);
    }

    #[test]
    fn never_drops_below_last_message() {
        let huge = "x".repeat(10_000);
        let msgs = vec![
            text_msg(Role::User, &huge),
            text_msg(Role::Assistant, &huge),
            text_msg(Role::User, &huge),
        ];
        let out = compact(&msgs, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], msgs[2]);
    }

    #[test]
    fn preserves_order() {
        let msgs = vec![
            text_msg(Role::User, "one"),
            text_msg(Role::Assistant, "two"),
            text_msg(Role::User, "three"),
            text_msg(Role::Assistant, "four"),
            text_msg(Role::User, "five"),
        ];
        let out = compact(&msgs, estimate_messages_tokens(&msgs[3..]));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], msgs[3]);
        assert_eq!(out[1], msgs[4]);
    }

    #[test]
    fn compaction_reduces_total_tokens_monotonically() {
        let s = "x".repeat(28);
        let msgs: Vec<Message> = (0..10).map(|_| text_msg(Role::User, &s)).collect();
        let before = estimate_messages_tokens(&msgs);
        let out = compact(&msgs, 50);
        let after = estimate_messages_tokens(&out);
        assert!(after <= 50, "after={after} > 50");
        assert!(
            after < before,
            "did not reduce: before={before} after={after}"
        );
    }

    #[test]
    fn render_for_summary_formats_messages() {
        let msgs = vec![
            text_msg(Role::User, "hello"),
            text_msg(Role::Assistant, "hi there"),
        ];
        let rendered = render_for_summary(&msgs);
        assert!(rendered.contains("User: hello"));
        assert!(rendered.contains("Assistant: hi there"));
    }
}
