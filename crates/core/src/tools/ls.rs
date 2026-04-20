use super::Tool;
use crate::error::{Error, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct LsTool;

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &'static str {
        "Ls"
    }

    fn description(&self) -> &'static str {
        "List the immediate contents of a directory (files and subdirectories, \
         non-recursive). Use this for `list files` requests and general \
         directory exploration. For recursive pattern matching use `Glob` \
         instead. Directories are suffixed with `/`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path (default: current working directory)"
                }
            }
        })
    }

    async fn call(&self, input: Value) -> Result<String> {
        let raw = input.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = crate::sandbox::Sandbox::check(raw)?;
        let entries = std::fs::read_dir(&path)
            .map_err(|e| Error::Tool(format!("ls {}: {e}", path.display())))?;

        let mut items: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            items.push(if is_dir { format!("{name}/") } else { name });
        }
        items.sort();
        Ok(items.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn lists_immediate_contents_non_recursive() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/deep.txt"), "").unwrap();

        let out = LsTool
            .call(json!({"path": dir.path().to_string_lossy()}))
            .await
            .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["a.txt", "b.txt", "sub/"], "got: {out}");
    }

    #[tokio::test]
    async fn empty_directory_returns_empty_string() {
        let dir = tempdir().unwrap();
        let out = LsTool
            .call(json!({"path": dir.path().to_string_lossy()}))
            .await
            .unwrap();
        assert_eq!(out, "");
    }

    #[tokio::test]
    async fn nonexistent_path_errors() {
        let err = LsTool
            .call(json!({"path": "/nope/does/not/exist"}))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("ls"));
    }

    #[tokio::test]
    async fn marks_subdirectories_with_trailing_slash() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("file"), "").unwrap();
        let out = LsTool
            .call(json!({"path": dir.path().to_string_lossy()}))
            .await
            .unwrap();
        assert!(out.contains("subdir/"));
        assert!(out.contains("file"));
        assert!(!out.contains("file/"));
    }
}
