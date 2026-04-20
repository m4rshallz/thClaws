//! TodoWrite tool — write/update the project's todo list.
//!
//! Stores todos in `.thclaws/todos.md` (markdown format).
//! Input: `{ "todos": [{"id": "1", "content": "Fix bug", "status": "in_progress"|"pending"|"completed"}] }`
//! The tool overwrites the entire todo list (full state replacement, not append).

use super::Tool;
use crate::error::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
}

impl TodoItem {
    /// Render as a markdown checklist line.
    fn to_markdown(&self) -> String {
        let checkbox = match self.status.as_str() {
            "completed" => "[x]",
            "in_progress" => "[-]",
            _ => "[ ]", // pending or unknown
        };
        format!("- {} {} (id: {})", checkbox, self.content, self.id)
    }
}

pub struct TodoWriteTool;

impl TodoWriteTool {
    fn todos_path() -> PathBuf {
        PathBuf::from(".thclaws").join("todos.md")
    }

    /// Write todos to a specific root directory (for testing).
    #[cfg(test)]
    fn write_todos_to(root: &std::path::Path, todos: &[TodoItem]) -> Result<String> {
        let path = root.join(".thclaws").join("todos.md");

        // Build markdown content.
        let mut md = String::from("# Todos\n\n");
        if todos.is_empty() {
            md.push_str("_No todos._\n");
        } else {
            for todo in todos {
                md.push_str(&todo.to_markdown());
                md.push('\n');
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Tool(format!("failed to create .thclaws dir: {e}")))?;
        }
        std::fs::write(&path, &md)
            .map_err(|e| Error::Tool(format!("failed to write todos.md: {e}")))?;

        let completed = todos.iter().filter(|t| t.status == "completed").count();
        let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
        let pending = todos.iter().filter(|t| t.status == "pending").count();

        Ok(format!(
            "Wrote {} todo(s) to .thclaws/todos.md ({} pending, {} in progress, {} completed)",
            todos.len(),
            pending,
            in_progress,
            completed,
        ))
    }

    fn parse_todos(input: &Value) -> Result<Vec<TodoItem>> {
        let todos_val = input
            .get("todos")
            .ok_or_else(|| Error::Tool("missing required field: todos".into()))?;

        serde_json::from_value(todos_val.clone())
            .map_err(|e| Error::Tool(format!("invalid todos array: {e}")))
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &'static str {
        "TodoWrite"
    }

    fn description(&self) -> &'static str {
        "Write or update the project's todo list. Overwrites the entire todo list with the provided items. \
         Stores todos in .thclaws/todos.md as a markdown checklist. \
         Each todo has an id, content string, and status (pending, in_progress, or completed)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete list of todo items. This replaces the entire existing list.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier for the todo item"
                            },
                            "content": {
                                "type": "string",
                                "description": "Description of the task"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Current status of the todo item"
                            }
                        },
                        "required": ["id", "content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    fn requires_approval(&self, _input: &Value) -> bool {
        true
    }

    async fn call(&self, input: Value) -> Result<String> {
        let todos = Self::parse_todos(&input)?;

        // Build markdown content.
        let mut md = String::from("# Todos\n\n");
        if todos.is_empty() {
            md.push_str("_No todos._\n");
        } else {
            for todo in &todos {
                md.push_str(&todo.to_markdown());
                md.push('\n');
            }
        }

        // Write to .thclaws/todos.md (relative to cwd).
        let path = Self::todos_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Tool(format!("failed to create .thclaws dir: {e}")))?;
        }
        std::fs::write(&path, &md)
            .map_err(|e| Error::Tool(format!("failed to write todos.md: {e}")))?;

        let completed = todos.iter().filter(|t| t.status == "completed").count();
        let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
        let pending = todos.iter().filter(|t| t.status == "pending").count();

        Ok(format!(
            "Wrote {} todo(s) to .thclaws/todos.md ({} pending, {} in progress, {} completed)",
            todos.len(),
            pending,
            in_progress,
            completed,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_todos_creates_markdown() {
        let dir = tempfile::tempdir().unwrap();

        let todos = vec![
            TodoItem {
                id: "1".into(),
                content: "Fix bug".into(),
                status: "completed".into(),
            },
            TodoItem {
                id: "2".into(),
                content: "Add tests".into(),
                status: "in_progress".into(),
            },
            TodoItem {
                id: "3".into(),
                content: "Deploy".into(),
                status: "pending".into(),
            },
        ];

        let result = TodoWriteTool::write_todos_to(dir.path(), &todos).unwrap();
        assert!(result.contains("3 todo(s)"));
        assert!(result.contains("1 pending"));
        assert!(result.contains("1 in progress"));
        assert!(result.contains("1 completed"));

        let contents = std::fs::read_to_string(dir.path().join(".thclaws/todos.md")).unwrap();
        assert!(contents.contains("# Todos"));
        assert!(contents.contains("[x] Fix bug (id: 1)"));
        assert!(contents.contains("[-] Add tests (id: 2)"));
        assert!(contents.contains("[ ] Deploy (id: 3)"));
    }

    #[tokio::test]
    async fn write_empty_todos() {
        let dir = tempfile::tempdir().unwrap();

        let result = TodoWriteTool::write_todos_to(dir.path(), &[]).unwrap();
        assert!(result.contains("0 todo(s)"));

        let contents = std::fs::read_to_string(dir.path().join(".thclaws/todos.md")).unwrap();
        assert!(contents.contains("_No todos._"));
    }

    #[tokio::test]
    async fn overwrites_existing_todos() {
        let dir = tempfile::tempdir().unwrap();

        // First write
        let todos1 = vec![TodoItem {
            id: "1".into(),
            content: "Old task".into(),
            status: "pending".into(),
        }];
        TodoWriteTool::write_todos_to(dir.path(), &todos1).unwrap();

        // Second write (full replacement)
        let todos2 = vec![TodoItem {
            id: "2".into(),
            content: "New task".into(),
            status: "completed".into(),
        }];
        TodoWriteTool::write_todos_to(dir.path(), &todos2).unwrap();

        let contents = std::fs::read_to_string(dir.path().join(".thclaws/todos.md")).unwrap();
        assert!(!contents.contains("Old task"));
        assert!(contents.contains("[x] New task (id: 2)"));
    }

    #[tokio::test]
    async fn missing_todos_field_errors() {
        let tool = TodoWriteTool;
        let err = tool.call(json!({})).await.unwrap_err();
        assert!(format!("{err}").contains("missing required field"));
    }

    #[test]
    fn tool_metadata() {
        let tool = TodoWriteTool;
        assert_eq!(tool.name(), "TodoWrite");
        assert!(tool.requires_approval(&json!({})));
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["todos"].is_object());
    }

    #[test]
    fn todo_item_markdown_rendering() {
        let completed = TodoItem {
            id: "1".into(),
            content: "Done".into(),
            status: "completed".into(),
        };
        assert_eq!(completed.to_markdown(), "- [x] Done (id: 1)");

        let in_prog = TodoItem {
            id: "2".into(),
            content: "Working".into(),
            status: "in_progress".into(),
        };
        assert_eq!(in_prog.to_markdown(), "- [-] Working (id: 2)");

        let pending = TodoItem {
            id: "3".into(),
            content: "Later".into(),
            status: "pending".into(),
        };
        assert_eq!(pending.to_markdown(), "- [ ] Later (id: 3)");
    }

    #[test]
    fn parse_todos_from_json() {
        let input = json!({
            "todos": [
                {"id": "1", "content": "Test", "status": "pending"}
            ]
        });
        let todos = TodoWriteTool::parse_todos(&input).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, "1");
        assert_eq!(todos[0].content, "Test");
        assert_eq!(todos[0].status, "pending");
    }
}
