//! Manifest schema for GUI Shells.
//!
//! Every shell — built-in, user-installed, project-installed — ships a
//! `manifest.json` with these fields. The picker (Tier 2) reads them
//! for display; the bridge (Tier 3) reads `permissions` for gating.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Bridge ABI version the shell was written against. Tier 1 ships
    /// version "1"; bumps happen if a method is ever removed (we plan
    /// to keep the surface additive — semver-minor for new methods,
    /// semver-major only for removals).
    #[serde(default = "default_bridge_version")]
    pub min_bridge_version: String,
    /// Coarse permission strings. Examples: `"agent.run"`,
    /// `"tools.invoke:image_gen"`, `"session.read"`,
    /// `"fs.shell-scoped"`, `"network.outbound:example.com"`. Tier 1
    /// stores but does not enforce; Tier 3 enforces.
    #[serde(default)]
    pub permissions: Vec<String>,
}

fn default_bridge_version() -> String {
    "1".to_string()
}

/// Permission strings the authoring tools accept. `tools.invoke:<t>` and
/// `network.outbound:<host>` are prefix forms (any suffix allowed).
const ALLOWED_PERMISSION_PREFIXES: &[&str] = &[
    "agent.run",
    "session.read",
    "session.list",
    "fs.shell-scoped",
    "tools.invoke:",
    "network.outbound:",
];

impl ShellManifest {
    /// Validate a manifest built from tool input before it's written to
    /// disk. Returns a human-readable error on the first problem so the
    /// model can fix and retry, rather than producing a shell that fails
    /// silently at load time.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if !is_kebab_id(&self.id) {
            return Err(format!(
                "invalid id '{}': must be lowercase kebab-case (a-z, 0-9, '-'), no slashes",
                self.id
            ));
        }
        for (field, val) in [
            ("name", &self.name),
            ("version", &self.version),
            ("entry", &self.entry),
        ] {
            if val.trim().is_empty() {
                return Err(format!("'{field}' must not be empty"));
            }
        }
        if self.entry.contains('/') || self.entry.contains("..") {
            return Err(format!(
                "entry '{}' must be a bare filename inside the shell folder",
                self.entry
            ));
        }
        for p in &self.permissions {
            let ok = ALLOWED_PERMISSION_PREFIXES.iter().any(|prefix| {
                if prefix.ends_with(':') {
                    p.starts_with(prefix) && p.len() > prefix.len()
                } else {
                    p == prefix
                }
            });
            if !ok {
                return Err(format!(
                    "unknown permission '{p}'. Allowed: agent.run, session.read, session.list, \
                     fs.shell-scoped, tools.invoke:<tool>, network.outbound:<host>"
                ));
            }
        }
        Ok(())
    }
}

/// Lowercase kebab id: non-empty, no leading/trailing dash, only
/// `[a-z0-9-]`. Keeps ids safe as a single path segment.
fn is_kebab_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_minimal_manifest() {
        let json = r#"{
            "id": "session-explorer",
            "name": "Session Explorer",
            "version": "0.1.0",
            "description": "Tree-view past sessions.",
            "entry": "index.html",
            "permissions": ["agent.run", "session.read"]
        }"#;
        let m: ShellManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "session-explorer");
        assert_eq!(m.min_bridge_version, "1");
        assert_eq!(m.permissions.len(), 2);
    }
}
