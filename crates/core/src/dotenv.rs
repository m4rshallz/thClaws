//! Minimal `.env` loader. Reads `KEY=VALUE` lines and sets them as env vars
//! if they aren't already set (existing env takes precedence).
//!
//! Load order (earlier wins):
//!   1. Process environment (explicit `export` in shell)
//!   2. `./.env` (project-local)
//!   3. `~/.config/thclaws/.env` (global)
//!
//! Lines starting with `#` are comments. Empty lines are skipped. Values
//! can be optionally quoted with `"` or `'` (quotes are stripped).

use std::path::{Path, PathBuf};

/// Load `.env` files from standard locations. Call once at startup before
/// any config or provider code reads env vars.
pub fn load_dotenv() {
    let candidates = [global_dotenv_path(), Some(PathBuf::from(".env"))];
    // Load global first, then project-local. Since we only set vars that
    // aren't already present, project-local effectively overrides global
    // because it's loaded second (and the first load set them).
    // Wait — that's backwards. We want project-local to win over global.
    // So load global first (sets unset vars), then project-local (can't
    // override because vars are already set). That's wrong.
    //
    // Fix: load project-local FIRST, then global. Already-set vars are
    // skipped, so project-local values stick.
    for path in candidates.into_iter().rev().flatten() {
        load_file(&path);
    }
}

fn global_dotenv_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config/thclaws/.env"))
}

/// Resolve the user-scope `.env` path, exposed so higher layers can
/// write to it (the keychain-fallback path in the Settings modal).
pub fn user_dotenv_path() -> Option<PathBuf> {
    global_dotenv_path()
}

/// Upsert a `KEY=value` line into the user-scope `~/.config/thclaws/.env`.
/// Creates the file if it doesn't exist. Preserves surrounding lines,
/// comments, and formatting — only the matching key's value is changed
/// (or the line appended if no match).
///
/// Used as a fallback when the OS keychain isn't available (headless
/// Linux without Secret Service, or a user who declined the keychain
/// permission prompt on macOS).
pub fn upsert_user_env(var: &str, value: &str) -> crate::error::Result<PathBuf> {
    let path =
        user_dotenv_path().ok_or_else(|| crate::error::Error::Config("HOME is not set".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(String::from).collect();
    let mut replaced = false;
    let prefix = format!("{var}=");
    for line in lines.iter_mut() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(&prefix) {
            *line = format!("{var}={value}");
            replaced = true;
            break;
        }
    }
    if !replaced {
        lines.push(format!("{var}={value}"));
    }
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&path, out)?;
    Ok(path)
}

/// Remove a `KEY=...` line from the user-scope `.env`. Silently succeeds
/// if the file or the key doesn't exist.
pub fn remove_from_user_env(var: &str) -> crate::error::Result<Option<PathBuf>> {
    let Some(path) = user_dotenv_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let existing = std::fs::read_to_string(&path)?;
    let prefix = format!("{var}=");
    let out: String = existing
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with(&prefix)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = out;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&path, out)?;
    Ok(Some(path))
}

fn load_file(path: &Path) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let mut value = value.trim();

        // Strip optional quotes.
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            value = &value[1..value.len() - 1];
        }

        // Only set if not already in the environment.
        if std::env::var(key).is_err() {
            std::env::set_var(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_file_sets_unset_vars() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "# comment\nTEST_DOTENV_A=hello\nTEST_DOTENV_B=\"quoted\"\n",
        )
        .unwrap();

        // Make sure they're not already set.
        std::env::remove_var("TEST_DOTENV_A");
        std::env::remove_var("TEST_DOTENV_B");

        load_file(&path);

        assert_eq!(std::env::var("TEST_DOTENV_A").unwrap(), "hello");
        assert_eq!(std::env::var("TEST_DOTENV_B").unwrap(), "quoted");

        // Cleanup.
        std::env::remove_var("TEST_DOTENV_A");
        std::env::remove_var("TEST_DOTENV_B");
    }

    #[test]
    fn existing_env_takes_precedence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "TEST_DOTENV_C=from_file\n").unwrap();

        std::env::set_var("TEST_DOTENV_C", "from_shell");
        load_file(&path);
        assert_eq!(std::env::var("TEST_DOTENV_C").unwrap(), "from_shell");

        std::env::remove_var("TEST_DOTENV_C");
    }
}
