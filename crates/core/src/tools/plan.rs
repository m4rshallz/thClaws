//! Plan mode tools: EnterPlanMode / ExitPlanMode.
//!
//! These are hint-based — they return a message that tells the model to
//! switch behavior. No server-side enforcement; the model cooperates
//! voluntarily. This matches the Claude Code pattern where the model
//! understands plan mode from its own history context.

use super::Tool;
use crate::error::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct EnterPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &'static str {
        "EnterPlanMode"
    }
    fn description(&self) -> &'static str {
        "Enter planning mode. While active, describe your plan step by step \
         without executing any tools. The user will review the plan and ask \
         you to call ExitPlanMode to proceed with execution."
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _input: Value) -> Result<String> {
        Ok(
            "Plan mode activated. I will now describe my plan step by step \
            without executing any tools. Call ExitPlanMode when you are ready \
            to proceed with execution."
                .into(),
        )
    }
}

pub struct ExitPlanModeTool;

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &'static str {
        "ExitPlanMode"
    }
    fn description(&self) -> &'static str {
        "Exit planning mode and begin executing the previously described plan. \
         Only call this after the user has reviewed and approved the plan."
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _input: Value) -> Result<String> {
        Ok("Plan mode deactivated. Proceeding with execution.".into())
    }
}
