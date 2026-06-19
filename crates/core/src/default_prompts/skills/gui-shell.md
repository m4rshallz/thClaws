---
name: gui-shell
short_description: Scaffold a custom GUI Shell (HTML frontend) on top of thClaws
description: Build a custom HTML frontend ("GUI Shell") that runs as a sandboxed tab inside the thClaws desktop GUI and talks to the agent via the window.thclaws.* bridge. Invoke when the user asks to "build a UI for …", "make a custom view / dashboard / form / gallery", or "scaffold a frontend on top of thClaws" — they mean a GUI Shell. This skill enables the GuiShellCreate / GuiShellWriteFile / GuiShellList / GuiShellRemove tools.
tool-gate: gui-shell
---

# GUI Shells (authoring)

A **GUI Shell** is a custom HTML frontend the user opens as a tab inside the thClaws desktop GUI. It runs in a sandboxed iframe and talks to the agent through a `window.thclaws.*` bridge. When the user asks to "build a UI for X", "make a custom view / dashboard / form / gallery", or "scaffold a frontend on top of thClaws", build one of these.

## Use the tools — do NOT hand-write files

Invoking this skill enabled four tools. Use them instead of `Write`-ing into the shell folder yourself — they validate the manifest, jail writes to the shell folder, and resolve scope/paths for you:

- **`GuiShellCreate`** — scaffold a new shell. Pass `id` (kebab-case), `name`, `description`, `entry_html` (the full index.html), optional `permissions[]`, and optional `files{}` (relpath → content for style.css / main.js / icon.svg / AGENTS.md). Validates and writes the folder. Pick `scope`: `"project"` (./.thclaws/gui-shell/, this repo) or `"user"` (~/.config/thclaws/gui-shell/, all projects). Project wins on id clash.
- **`GuiShellWriteFile`** — iterate on one file (`id`, `relpath`, `content`). Refuses manifest.json — change the manifest by re-running `GuiShellCreate`.
- **`GuiShellList`** — show installed shells.
- **`GuiShellRemove`** — delete a shell (asks for approval).

## Manifest fields (passed as `GuiShellCreate` params, not raw JSON)

`id`, `name`, `version` (default 0.1.0), `description`, `entry` (default index.html), `icon`, and `permissions`. Permissions are declared up front and enforced at call time:

- `agent.run` — run the agent loop
- `tools.invoke:<tool-name>` — direct deterministic tool invocation
- `session.read` / `session.list` — read sidecar session data
- `fs.shell-scoped` — read/write inside the shell folder
- `network.outbound:<host>` — `fetch()` to that host (CSP injected)

Anything not declared throws. Request the minimum the shell needs.

## Bridge — `window.thclaws.*`

The bridge is the ONLY API the shell has. The iframe sandbox blocks direct workspace fs access, cross-shell storage leaks, and arbitrary network egress.

```js
// identity
thclaws.shell.id; thclaws.shell.sessionId; thclaws.transport;  // 'tauri' | 'ws'

// agent loop (same engine as Chat/Terminal)
const { runId } = await thclaws.run("user message");
thclaws.cancel(runId);
thclaws.on("text" | "tool_call" | "tool_result" | "done" | "error", cb);

// direct tool invocation — skips the model, deterministic
const out = await thclaws.tools.invoke("ToolName", args);

// per-shell, per-session persistence (file-backed JSON)
await thclaws.storage.set(key, value);
const v = await thclaws.storage.get(key);
```

You don't ship the bridge — it's injected into every shell's `<head>` at serve time.

## Install + iterate

1. `GuiShellCreate` writes the folder.
2. GUI → "+ New Tab" → "GUI Shell" → click **Refresh shells** → the new tile appears.
3. Click the tile to open. No restart; the bridge is injected at iframe load.
4. After edits with `GuiShellWriteFile`, close + reopen the shell tab (no hot-reload in v1).

Set a project default in `.thclaws/settings.json`: `{ "guiShell": "<id>" }` — "+ New Tab" then opens this shell directly instead of the picker.

## Reference shells

Source-bundled at `<thclaws-source>/crates/core/assets/gui-shells/`:
- `chatbot/` — minimal `thclaws.run()` + `thclaws.storage` demo (~120 LOC)
- `session-explorer/` — tree-of-sessions browser with on-demand summaries

Full reference: user-manual chapter 26 (`ch26-gui-shells`).
