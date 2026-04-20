//! Sub-agent tool — spawn nested agents with depth tracking and
//! named agent definitions.
//!
//! Supports multi-level recursion up to `max_depth` (default 3).
//! Child agents include their own `Task` tool at `depth + 1`, so
//! they can delegate further. At max depth, the tool refuses.
//!
//! Named agents: if `agent` field is provided in the input, loads
//! the definition from `~/.config/thclaws/agents.json` and uses
//! its instructions, model override, and tool subset.

use crate::agent::{collect_agent_turn, Agent};
use crate::agent_defs::AgentDef;
use crate::error::{Error, Result};
use crate::tools::{req_str, Tool};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub const TOOL_NAME: &str = "Task";
pub const DEFAULT_MAX_DEPTH: usize = 3;

/// How to construct a child agent. Implementations produce a brand-new
/// `Agent` with the appropriate configuration.
#[async_trait]
pub trait AgentFactory: Send + Sync {
    /// Build a child agent. `agent_def` is `Some` if the Task input
    /// specified a named agent; `None` for the default.
    async fn build(
        &self,
        prompt: &str,
        agent_def: Option<&AgentDef>,
        child_depth: usize,
    ) -> Result<Agent>;
}

pub struct SubAgentTool {
    factory: Arc<dyn AgentFactory>,
    depth: usize,
    max_depth: usize,
    /// Agent definitions loaded at startup.
    agent_defs: crate::agent_defs::AgentDefsConfig,
}

impl SubAgentTool {
    pub fn new(factory: Arc<dyn AgentFactory>) -> Self {
        Self {
            factory,
            depth: 0,
            max_depth: DEFAULT_MAX_DEPTH,
            agent_defs: crate::agent_defs::AgentDefsConfig::load_with_extra(
                &crate::plugins::plugin_agent_dirs(),
            ),
        }
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    pub fn with_agent_defs(mut self, defs: crate::agent_defs::AgentDefsConfig) -> Self {
        self.agent_defs = defs;
        self
    }
}

#[async_trait]
impl Tool for SubAgentTool {
    fn name(&self) -> &'static str {
        TOOL_NAME
    }

    fn description(&self) -> &'static str {
        "Launch a sub-agent with its own history to handle a bounded subtask. \
         The sub-agent runs independently, may call tools (and spawn further \
         sub-agents up to the recursion limit), and returns its final response \
         as text. Use `agent` to pick a named agent definition from agents.json."
    }

    fn input_schema(&self) -> Value {
        let mut agent_names = self.agent_defs.names();
        agent_names.sort();
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Short label for the sub-task (shown in logs)."
                },
                "prompt": {
                    "type": "string",
                    "description": "The full instruction for the sub-agent."
                },
                "agent": {
                    "type": "string",
                    "description": format!(
                        "Optional named agent from agents.json. Available: {}",
                        if agent_names.is_empty() { "none configured".to_string() }
                        else { agent_names.join(", ") }
                    )
                }
            },
            "required": ["prompt"]
        })
    }

    fn requires_approval(&self, _input: &Value) -> bool {
        true
    }

    async fn call(&self, input: Value) -> Result<String> {
        if self.depth >= self.max_depth {
            return Err(Error::Agent(format!(
                "sub-agent recursion limit reached (depth {}/{})",
                self.depth, self.max_depth
            )));
        }

        let prompt = req_str(&input, "prompt")?.to_string();
        let agent_name = input.get("agent").and_then(Value::as_str);

        // Look up named agent definition if specified.
        let agent_def = agent_name.and_then(|name| self.agent_defs.get(name));
        if agent_name.is_some() && agent_def.is_none() {
            let available = self.agent_defs.names().join(", ");
            return Err(Error::Agent(format!(
                "unknown agent '{}'. Available: {}",
                agent_name.unwrap(),
                if available.is_empty() {
                    "none"
                } else {
                    &available
                }
            )));
        }

        let child_depth = self.depth + 1;
        let agent = self.factory.build(&prompt, agent_def, child_depth).await?;
        let stream = agent.run_turn(prompt);
        let outcome = collect_agent_turn(stream).await?;

        if outcome.text.is_empty() {
            Err(Error::Agent("sub-agent returned empty response".into()))
        } else {
            Ok(outcome.text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentEvent;
    use crate::error::Error;
    use crate::providers::{EventStream, Provider, ProviderEvent, StreamRequest};
    use crate::tools::ToolRegistry;
    use async_trait::async_trait;
    use futures::{stream, StreamExt};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct ScriptedProvider {
        scripts: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    }

    impl ScriptedProvider {
        fn new(scripts: Vec<Vec<ProviderEvent>>) -> Arc<Self> {
            Arc::new(Self {
                scripts: Arc::new(Mutex::new(VecDeque::from(scripts))),
            })
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn stream(&self, _req: StreamRequest) -> Result<EventStream> {
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| Error::Provider("no more scripts".into()))?;
            let events: Vec<Result<ProviderEvent>> = script.into_iter().map(Ok).collect();
            Ok(Box::pin(stream::iter(events)))
        }
    }

    fn text_script(chunks: &[&str]) -> Vec<ProviderEvent> {
        let mut out = vec![ProviderEvent::MessageStart {
            model: "test".into(),
        }];
        for c in chunks {
            out.push(ProviderEvent::TextDelta((*c).to_string()));
        }
        out.push(ProviderEvent::ContentBlockStop);
        out.push(ProviderEvent::MessageStop {
            stop_reason: Some("end_turn".into()),
            usage: None,
        });
        out
    }

    struct SimpleFactory {
        scripts: Arc<Mutex<Vec<Vec<Vec<ProviderEvent>>>>>,
    }

    impl SimpleFactory {
        fn new(scripts: Vec<Vec<Vec<ProviderEvent>>>) -> Arc<Self> {
            Arc::new(Self {
                scripts: Arc::new(Mutex::new(scripts)),
            })
        }
    }

    #[async_trait]
    impl AgentFactory for SimpleFactory {
        async fn build(
            &self,
            _prompt: &str,
            _def: Option<&AgentDef>,
            _depth: usize,
        ) -> Result<Agent> {
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| Error::Agent("factory exhausted".into()))?;
            let provider = ScriptedProvider::new(script);
            Ok(Agent::new(provider, ToolRegistry::new(), "test", ""))
        }
    }

    #[tokio::test]
    async fn sub_agent_returns_text() {
        let factory = SimpleFactory::new(vec![vec![text_script(&["done"])]]);
        let tool = SubAgentTool::new(factory);
        let out = tool.call(json!({"prompt": "go"})).await.unwrap();
        assert_eq!(out, "done");
    }

    #[tokio::test]
    async fn depth_limit_enforced() {
        let factory = SimpleFactory::new(vec![]);
        let tool = SubAgentTool::new(factory).with_depth(3).with_max_depth(3);
        let err = tool.call(json!({"prompt": "go"})).await.unwrap_err();
        assert!(format!("{err}").contains("recursion limit"));
    }

    #[tokio::test]
    async fn unknown_agent_errors() {
        let factory = SimpleFactory::new(vec![]);
        let tool = SubAgentTool::new(factory);
        let err = tool
            .call(json!({"prompt": "go", "agent": "nonexistent"}))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown agent"));
    }

    #[tokio::test]
    async fn named_agent_passed_to_factory() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let saw_def = Arc::new(AtomicBool::new(false));
        let saw_def_clone = saw_def.clone();

        struct DefCheckFactory(Arc<AtomicBool>);
        #[async_trait]
        impl AgentFactory for DefCheckFactory {
            async fn build(&self, _p: &str, def: Option<&AgentDef>, _d: usize) -> Result<Agent> {
                if let Some(d) = def {
                    assert_eq!(d.name, "researcher");
                    self.0.store(true, Ordering::Relaxed);
                }
                let provider = ScriptedProvider::new(vec![text_script(&["found it"])]);
                Ok(Agent::new(provider, ToolRegistry::new(), "test", ""))
            }
        }

        let defs = crate::agent_defs::AgentDefsConfig {
            agents: vec![AgentDef {
                name: "researcher".into(),
                instructions: "Research things".into(),
                max_iterations: 5,
                ..Default::default()
            }],
        };

        let factory = Arc::new(DefCheckFactory(saw_def_clone));
        let tool = SubAgentTool::new(factory).with_agent_defs(defs);
        let out = tool
            .call(json!({"prompt": "find X", "agent": "researcher"}))
            .await
            .unwrap();
        assert_eq!(out, "found it");
        assert!(saw_def.load(Ordering::Relaxed));
    }
}
