//! Configurable prompt templates.
//!
//! Every user-facing prompt used by the agent can be overridden by dropping a
//! markdown file into `.thclaws/prompt/<name>.md` (project level) or
//! `~/.config/thclaws/prompt/<name>.md` (user level). Project wins over user;
//! both win over the built-in default.
//!
//! Templates support `{variable}` substitution. Unknown placeholders are left
//! untouched so users notice typos.

use std::path::PathBuf;

const DIR: &str = "prompt";

/// Built-in default templates. These are the bytes of the markdown files under
/// `src/default_prompts/`, embedded at compile time. The same files should
/// serve as the canonical reference for authors writing overrides into
/// `.thclaws/prompt/`.
pub mod defaults {
    pub const SYSTEM: &str = include_str!("default_prompts/system.md");
    pub const LEAD: &str = include_str!("default_prompts/lead.md");
    pub const AGENT_TEAM: &str = include_str!("default_prompts/agent_team.md");
    pub const SUBAGENT: &str = include_str!("default_prompts/subagent.md");
    pub const WORKTREE: &str = include_str!("default_prompts/worktree.md");
    pub const COMPACTION: &str = include_str!("default_prompts/compaction.md");
    pub const COMPACTION_SYSTEM: &str = include_str!("default_prompts/compaction_system.md");
}

fn project_path(name: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".thclaws")
        .join(DIR)
        .join(format!("{name}.md"))
}

fn user_path(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("thclaws").join(DIR).join(format!("{name}.md")))
}

/// Load a prompt template by name. Returns the override content (project →
/// user) if present, otherwise the built-in default string.
pub fn load(name: &str, default: &str) -> String {
    if let Ok(s) = std::fs::read_to_string(project_path(name)) {
        return s;
    }
    if let Some(p) = user_path(name) {
        if let Ok(s) = std::fs::read_to_string(p) {
            return s;
        }
    }
    default.to_string()
}

/// Replace `{key}` occurrences with the corresponding values. Unknown
/// placeholders are left in place so typos are visible.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Load-and-render in one call.
pub fn render_named(name: &str, default: &str, vars: &[(&str, &str)]) -> String {
    render(&load(name, default), vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_known_keys() {
        let out = render(
            "hello {name}, you are {role}",
            &[("name", "ada"), ("role", "lead")],
        );
        assert_eq!(out, "hello ada, you are lead");
    }

    #[test]
    fn render_leaves_unknown_keys_alone() {
        let out = render("hi {name} — {missing}", &[("name", "ada")]);
        assert_eq!(out, "hi ada — {missing}");
    }

    #[test]
    fn load_falls_back_to_default_when_no_override() {
        let out = load("__nonexistent_prompt_xyz__", "DEFAULT");
        assert_eq!(out, "DEFAULT");
    }
}
