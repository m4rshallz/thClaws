//! In-memory task/todo system: TaskCreate, TaskUpdate, TaskGet, TaskList.
//! Shared state via `Arc<Mutex<TaskStore>>` across all four tools.

use super::{req_str, Tool};
use crate::error::{Error, Result};
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: String,
}

#[derive(Debug, Default)]
pub struct TaskStore {
    tasks: Vec<Task>,
    next_id: u64,
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn create(&mut self, subject: String, description: String) -> Task {
        self.next_id += 1;
        let task = Task {
            id: format!("{}", self.next_id),
            subject,
            description,
            status: "pending".into(),
        };
        self.tasks.push(task.clone());
        task
    }

    fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    fn update(
        &mut self,
        id: &str,
        status: Option<&str>,
        subject: Option<&str>,
        desc: Option<&str>,
    ) -> Option<Task> {
        let task = self.tasks.iter_mut().find(|t| t.id == id)?;
        if let Some(s) = status {
            task.status = s.to_string();
        }
        if let Some(s) = subject {
            task.subject = s.to_string();
        }
        if let Some(d) = desc {
            task.description = d.to_string();
        }
        Some(task.clone())
    }

    pub fn list(&self) -> &[Task] {
        &self.tasks
    }
}

fn format_task(t: &Task) -> String {
    format!(
        "#{} [{}] {}\n  {}",
        t.id, t.status, t.subject, t.description
    )
}

pub type SharedTaskStore = Arc<Mutex<TaskStore>>;

/// Register all four task tools into the given registry. Returns the shared
/// store so the REPL can access it for `/tasks` display.
pub fn register_task_tools(registry: &mut ToolRegistry) -> SharedTaskStore {
    let store: SharedTaskStore = Arc::new(Mutex::new(TaskStore::new()));
    registry.register(Arc::new(TaskCreateTool(store.clone())));
    registry.register(Arc::new(TaskUpdateTool(store.clone())));
    registry.register(Arc::new(TaskGetTool(store.clone())));
    registry.register(Arc::new(TaskListTool(store.clone())));
    store
}

// ---------- TaskCreate ----------

pub struct TaskCreateTool(SharedTaskStore);

#[async_trait]
impl Tool for TaskCreateTool {
    fn name(&self) -> &'static str {
        "TaskCreate"
    }
    fn description(&self) -> &'static str {
        "Create a new task/todo item. Returns the created task with its id."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subject": {"type": "string", "description": "Brief title for the task"},
                "description": {"type": "string", "description": "What needs to be done"}
            },
            "required": ["subject", "description"]
        })
    }
    async fn call(&self, input: Value) -> Result<String> {
        let subject = req_str(&input, "subject")?.to_string();
        let desc = req_str(&input, "description")?.to_string();
        let task = self.0.lock().unwrap().create(subject, desc);
        Ok(format!("Task #{} created: {}", task.id, task.subject))
    }
}

// ---------- TaskUpdate ----------

pub struct TaskUpdateTool(SharedTaskStore);

#[async_trait]
impl Tool for TaskUpdateTool {
    fn name(&self) -> &'static str {
        "TaskUpdate"
    }
    fn description(&self) -> &'static str {
        "Update a task's status, subject, or description. Status values: pending, in_progress, completed."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]},
                "subject": {"type": "string"},
                "description": {"type": "string"}
            },
            "required": ["id"]
        })
    }
    async fn call(&self, input: Value) -> Result<String> {
        let id = req_str(&input, "id")?;
        let status = input.get("status").and_then(Value::as_str);
        let subject = input.get("subject").and_then(Value::as_str);
        let desc = input.get("description").and_then(Value::as_str);
        match self.0.lock().unwrap().update(id, status, subject, desc) {
            Some(t) => Ok(format_task(&t)),
            None => Err(Error::Tool(format!("task not found: {id}"))),
        }
    }
}

// ---------- TaskGet ----------

pub struct TaskGetTool(SharedTaskStore);

#[async_trait]
impl Tool for TaskGetTool {
    fn name(&self) -> &'static str {
        "TaskGet"
    }
    fn description(&self) -> &'static str {
        "Get a task by id. Returns subject, description, and status."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"}
            },
            "required": ["id"]
        })
    }
    async fn call(&self, input: Value) -> Result<String> {
        let id = req_str(&input, "id")?;
        let store = self.0.lock().unwrap();
        match store.get(id) {
            Some(t) => Ok(format_task(t)),
            None => Err(Error::Tool(format!("task not found: {id}"))),
        }
    }
}

// ---------- TaskList ----------

pub struct TaskListTool(SharedTaskStore);

#[async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &'static str {
        "TaskList"
    }
    fn description(&self) -> &'static str {
        "List all tasks with their id, status, subject, and description."
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn call(&self, _input: Value) -> Result<String> {
        let store = self.0.lock().unwrap();
        let tasks = store.list();
        if tasks.is_empty() {
            return Ok("No tasks.".into());
        }
        Ok(tasks
            .iter()
            .map(format_task)
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> SharedTaskStore {
        Arc::new(Mutex::new(TaskStore::new()))
    }

    #[tokio::test]
    async fn create_and_list() {
        let store = make_store();
        let create = TaskCreateTool(store.clone());
        let list = TaskListTool(store.clone());

        create
            .call(json!({"subject": "Fix bug", "description": "segfault on exit"}))
            .await
            .unwrap();
        create
            .call(json!({"subject": "Add tests", "description": "cover edge cases"}))
            .await
            .unwrap();

        let out = list.call(json!({})).await.unwrap();
        assert!(out.contains("#1"));
        assert!(out.contains("#2"));
        assert!(out.contains("Fix bug"));
        assert!(out.contains("Add tests"));
    }

    #[tokio::test]
    async fn get_by_id() {
        let store = make_store();
        let create = TaskCreateTool(store.clone());
        let get = TaskGetTool(store.clone());

        create
            .call(json!({"subject": "Task A", "description": "desc"}))
            .await
            .unwrap();
        let out = get.call(json!({"id": "1"})).await.unwrap();
        assert!(out.contains("Task A"));
        assert!(out.contains("pending"));
    }

    #[tokio::test]
    async fn update_status() {
        let store = make_store();
        let create = TaskCreateTool(store.clone());
        let update = TaskUpdateTool(store.clone());
        let get = TaskGetTool(store.clone());

        create
            .call(json!({"subject": "Do it", "description": "now"}))
            .await
            .unwrap();
        update
            .call(json!({"id": "1", "status": "completed"}))
            .await
            .unwrap();

        let out = get.call(json!({"id": "1"})).await.unwrap();
        assert!(out.contains("completed"));
    }

    #[tokio::test]
    async fn get_missing_errors() {
        let store = make_store();
        let get = TaskGetTool(store);
        let err = get.call(json!({"id": "999"})).await.unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[tokio::test]
    async fn register_adds_four_tools() {
        let mut reg = ToolRegistry::new();
        let _store = register_task_tools(&mut reg);
        assert!(reg.get("TaskCreate").is_some());
        assert!(reg.get("TaskUpdate").is_some());
        assert!(reg.get("TaskGet").is_some());
        assert!(reg.get("TaskList").is_some());
    }
}
