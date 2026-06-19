# Chapter 25 — Workflows

Workflows are thClaws's **fourth orchestration tier**: the model writes a
JavaScript script that fans work out across many subagents, and a
sandboxed JS engine runs that script deterministically on your
machine. Unlike subagents (Chapter 15), `/agent` side-channels, or
Agent Teams (Chapter 17), the orchestrator here is **code**, not the
model — which means rerunning the same workflow gives the same shape
of work every time and a long-running job leaves a checkpoint on disk.

Fan-out, JSON-schema validation, per-worker token/time budgets,
retries, KMS-write grants, and resume all work today. The main
remaining gap is **true parallel execution** — workers still run one
at a time even inside `Promise.all` (see "Current limitations" below).

## When to use workflows

Use workflows for **bulk, deterministic, mostly-independent work**:

- "rewrite all 800 test files to use the new fixture"
- "for every `.md` page under `kms/bug/`, translate it to Thai"
- "audit each crate's `Cargo.toml` and flag deprecated deps"

Use the `Task` tool (Chapter 15) for **one-shot model-driven
side-quests** the agent decides to spawn during a normal turn — that's
what subagents are still for.

Use `/agent` (Chapter 15) when **you** know exactly what a specialist
should do and want it running concurrently with your main session.

Use Agent Teams (Chapter 17) when teammates need to **collaborate**
— exchange messages, debate hypotheses, coordinate on a shared task
list. Workflows are for stateless fan-out; teams are for stateful
collaboration.

## Two ways to invoke a workflow

Same underlying engine (LLM authors a JS script, Boa sandbox runs
it); two different entry points depending on who decides a workflow
is the right tool:

| Trigger | Best for | UX |
|---|---|---|
| **You type `/workflow run <prompt>`** (the slash command) | You already know fan-out is the right shape and want to review the script before it runs | Author → review → approve/cancel/re-author loop → run |
| **The model calls the `WorkflowRun` tool** | You describe the work in natural language ("rewrite all the tests using the new fixture") and let the model decide whether to spawn one subagent or author a parallel workflow | Per-call approval prompt (same one Bash gets) → author → run, no review loop |

Pick the slash command when you want to see the JS before it
executes — useful for novel patterns or when you're iterating on the
shape of the work. Pick the tool path when you just want the result
and trust the model to choose the orchestration strategy.

The model knows about `WorkflowRun` because it's listed in the
**Collaboration primitives** section of the system prompt alongside
Subagent (single-shot side-quest) and Agent Teams (persistent
collaborators) — so it can pick the right primitive without being
told. If you want to nudge it explicitly, just say "use WorkflowRun
to …" in chat.

Both paths reject nested calls: a script that tries to invoke
`WorkflowRun` inside itself fails with a clear error — orchestrate
through `thclaws.subagent(...)` instead (the script's only fan-out
primitive; there is no `thclaws.parallel`).

## Quick start

```text
/workflow run summarize each .rs file under src/ in one line
```

What happens, in order:

1. **Author phase.** The model writes a JavaScript script using the
   `thclaws.*` API (the API is detailed in the model's system prompt,
   so the script you get back already knows what's available).
2. **Review.** The script is printed with line numbers. You're
   prompted:
   ```text
   [a]pprove · [c]ancel · [r]e-author:
   ```
   - `a` — run the script as written.
   - `c` — drop the workflow.
   - `r` — give a one-line revision note ("use the read tool not bash
     cat") and the model rewrites the script with that feedback. Loop
     until you `a` or `c`.
3. **Execute.** A workflow id (`wf-…`) prints, then each subagent
   invocation shows a progress line:
   ```text
   ✓ w0  List every .rs file under src/, recursively. Return o…   2s
   ✓ w1  Read crates/core/src/agent.rs and write ONE sentence …   3s
   ✓ w2  Read crates/core/src/repl.rs and write ONE sentence d…   4s
   …
   workflow done — 47 workers, total 1m 12s
   crates/core/src/agent.rs — the streaming agent loop
   crates/core/src/repl.rs — REPL command parser + rustyline I/O
   …
   ```

If a worker errors, you see `✗ wN  …` for that line and the script
typically catches and continues (depending on what the model wrote).

### Quick start via the model

Same task, no slash command — just ask in chat:

```text
you > rewrite each test file under tests/ to use the new TestHarness fixture
```

The model recognises this as fan-out and decides to call
`WorkflowRun`:

```text
[approval] WorkflowRun(prompt: "rewrite each test file under tests/
                                 to use the new TestHarness fixture")
[a]llow once · [A]lways · [d]eny: a
[workflow: author phase…]
[workflow: 32 subagent turn(s), 18432 in / 9621 out tokens]
✓ tests/test_login.rs — migrated to TestHarness::new()
✓ tests/test_signup.rs — migrated; 1 helper renamed
…
```

You see one approval prompt for the whole `WorkflowRun` call (the
same shape Bash gets); inside the script, individual `Skill` /
`Bash` / `Edit` calls each still need their own approvals as
normal — `WorkflowRun` doesn't bypass the per-tool gates inside.

If you want to see the JS script the model authored, use the
slash-command path instead (`/workflow run <same prompt>`). The
tool path skips the review loop for speed.

## The `thclaws.*` API

Your script gets exactly one global — `thclaws` — with these
fields:

```js
thclaws.subagent({
  prompt: string,           // required — the worker's task
  budget?: {                // both enforced
    time?: number | string, //   "60s" / "2m" / "1m30s" / 60 (seconds)
    tokens?: number,        //   input + output cap per worker call
  },
  schema?: object,          // JSON Schema. Worker is asked for JSON
                            //   matching the schema; on success the call
                            //   returns the parsed value (not text).
  retry?: number | {        // retries on hard errors + schema misses
    max: number,
    backoff?: string,       //   "exponential" / "linear" / "500ms" / etc.
  },
  caps?: {                  // explicit grants — default DENY for KMS writes
    kms?: { write?: string[] },
  },
  agent?: string,           // run as a named agent def (.thclaws/agents/<name>.md);
                            //   inherits its tools/instructions. See Chapter 15.
}) → string | parsed_value
```

The script also gets `thclaws.include(path)` — pull in another `.js`
file (helpers, shared prompt strings) relative to the script's
directory. Path traversal (`..`), absolute paths, and symlinks that
escape the base dir are rejected.

**`caps.kms.write` controls what a worker can write.** Outside a
workflow run KMS write tools work as before. Inside `/workflow run`,
workers default to **deny** for all KMS writes; the script must
pass `caps: { kms: { write: ["scratch", "audit-log"] } }` to grant
write access to specific KMS names for that single call. Grants are
audited as `worker_caps` events in state.jsonl, and they are NOT
transitive — a worker's own model-driven Task spawns get fresh
(empty) caps unless re-granted explicitly.

Time budget triggers `tokio::time::timeout`; exceeding it throws.
Schema validation runs after each attempt — if the worker's text isn't
parsable JSON or doesn't match the schema, the call retries up to
`retry.max` with the chosen `backoff`. Every retry is logged as a
`worker_retry` event so `/workflow inspect <id>` shows the journey. Workers inherit the parent session's provider,
model, system prompt, tool registry, memory, KMS, and permission mode
— so a worker can `Bash`, `Read`, `Edit`, search KMS, use MCP servers,
etc. Subagent recursion (a worker calling Task itself) is bounded by
the same `DEFAULT_MAX_DEPTH = 3` ceiling sub-agents already honour.

**Async syntax works** — scripts that use `await` / `async` /
`Promise.all` route through Boa Module mode. `thclaws.subagent` is
still synchronous internally, so `Promise.all([...])` resolves
correctly but workers execute in source order (one at a time).
Genuine parallel execution is still pending (see "Current
limitations").

### What you can write in the script

JS control flow: `for`, `while`, `if`/`else`, `try`/`catch`, `await`,
`async` functions, `Promise.all`, destructuring, template literals,
`Array` and `String` methods, regex, JSON parsing.

### What you can't write

- `eval`, `Function` (stripped from the sandbox)
- `fetch`, `require`, `process`, DOM, `console.log`

Anything I/O-flavoured must go through a subagent.

### A short example

```js
// Workflow: list .rs files, summarise each
const list = await thclaws.subagent({
  prompt: "List every .rs file under src/, recursively. Paths only."
});
const paths = list.split("\n").map(s => s.trim()).filter(Boolean);

const summaries = await Promise.all(
  paths.map(p => thclaws.subagent({
    prompt: `Read ${p} and write ONE sentence describing what it does.`
  }))
);

paths.map((p, i) => `${p} — ${summaries[i]}`).join("\n");
```

For sync scripts the **last expression** is the result. For async
scripts (which run in Module mode) the result is the last expression
the auto-wrapper finds, OR an explicit `globalThis.__wf_result = …`
assignment, OR `undefined` if neither.

## State on disk

Every run writes a JSONL log to:

```text
.thclaws/workflows/wf-<id>/state.jsonl
```

One event per line, flushed after each write so a Ctrl-C leaves the
file in a recoverable shape. Event shapes:

```jsonl
{"ts":"…","kind":"start","id":"wf-…","prompt":"…","script_sha":"…","script_chars":234}
{"ts":"…","kind":"worker_start","id":"wf-…","worker":"w0","prompt":"…"}
{"ts":"…","kind":"worker_done","id":"wf-…","worker":"w0","output":"…"}
{"ts":"…","kind":"worker_error","id":"wf-…","worker":"w1","error":"…"}
{"ts":"…","kind":"done","id":"wf-…","result":"…"}
```

You can `cat`, `grep`, or `jq` the file at any time — it's plain
JSONL, never opaque. The companion `script.js` (the approved JS) is
written next to it at start, so `/workflow resume <id>` can replay
against the same source.

Slash commands for managing runs (these work in both the CLI REPL and
the GUI chat tab, **except** `/workflow rm`, which needs an interactive
y/N confirm the chat tab can't show — run it from `thclaws --cli`):

```text
/workflow list             one line per run on disk, newest first
/workflow inspect <id>     dump state.jsonl events in human form
/workflow resume <id>      re-run, replaying completed workers from
                           the cache; fresh spawns continue numbering
/workflow rm <id>          y/N confirm + remove the workflow dir
```

`resume` is keyed by **prompt match** at each `thclaws.subagent`
call. A cached entry is consumed only when its prompt equals the
current call's prompt; mismatches fall through to fresh spawn (the
script may have been edited or paths changed). Any cached entries
left over at script end are reported as "diverged" so you know.

If `.thclaws/` can't be written (read-only volume, permissions), the
workflow runs anyway and prints:
```text
/workflow run: state.jsonl unavailable — proceeding without checkpoint
```
You lose the audit trail but not the run.

## Running a pre-authored script (skip the author phase)

When you already have a `.js` workflow on disk — one you wrote by hand,
or the `script.js` saved from an earlier `/workflow run` — you can run
it directly, skipping the author + review phase entirely:

- **Mid-session:** `/workflow exec <path>` (aliases `file` / `script`)
  runs the script from disk in the current REPL **or** GUI session.
- **Headless:** `thclaws --workflow <file.js>` runs it from the command
  line — useful for CI, cron jobs (chapter 19), and deploy hooks.

Every `/workflow run` persists its approved script to
`.thclaws/workflows/wf-<id>/script.js`, so a workflow the model
authored once can be re-run deterministically with `/workflow exec`
against that path (or replayed from its checkpoint with
`/workflow resume <id>`).

```sh
# Fresh run:
thclaws --workflow ./scripts/audit-crates.js

# Resume a previous workflow id (or unique prefix):
thclaws --workflow ./scripts/audit-crates.js --resume wf-18b3fa

# stdout = script's final value; stderr = id + done summary.
# Pipe stdout to jq, redirect to file, etc.:
thclaws --workflow ./scripts/audit-crates.js > result.txt
```

Exit code is 0 on success, 1 on script failure. **Both** `/workflow
exec` and `--workflow` auto-approve every tool call the workers make
(same as `--dangerously-skip-permissions`) — the script is treated as
operator-vetted, so only run scripts you trust.

> `thclaws -p "/workflow run <goal>"` is not a useful path: one-shot
> print mode has no surface for the author/review loop, and the
> `thclaws.subagent` host function isn't wired there. For
> non-interactive runs, pre-author a script and use `--workflow`.

## Current limitations

These are documented gaps, not bugs — tracked in
[dev-plan/32](../dev-plan/32-dynamic-workflows.md) (workspace-only):

- **`Promise.all` resolves but doesn't truly parallelise.** Boa runs
  scripts that use `await` / `Promise.all` in Module mode, so the
  syntax parses and `await thclaws.subagent(...)` returns the worker's
  text. But the host function still blocks the JS thread per-call, so
  each subagent call inside `Promise.all` runs sequentially. Wall-clock
  = sum of latencies, not max. A tokio-integrated executor for genuine
  parallelism is still pending.
- **No verification phase.** A dedicated `thclaws.verify({...})`
  primitive doesn't exist yet — scripts that want a check step author
  it as another `thclaws.subagent` call.
- **No real-time GUI worker grid.** `/workflow run` and the management
  commands work from the chat tab, but worker progress streams as text
  rather than a live grid. A real-time progress UI is still a frontend
  deliverable.

## Cost awareness

Each `thclaws.subagent` call is a separate model turn — typically a
few seconds and a few hundred to a few thousand tokens. A 200-worker
workflow can easily burn $5–$20 of API tokens depending on the model.
Two practical guardrails:

- **Cap the fan-out before writing the script.** If the goal is
  unbounded ("every file"), have a *discovery* subagent return the
  list first so you see the cardinality before approving the script.
- **Watch the close-out summary.** `workflow done — N workers, total
  Xs, In tokens / Out tokens (≈$Y.YY)` is printed after every
  `/workflow run`. Tier-billed or unknown models show `(cost unknown)`
  instead of a number.

## Quick reference

| | Subagent (`Task`) | `/agent` | Agent Teams | Workflow |
|---|---|---|---|---|
| Who orchestrates | The model | You (one-shot) | Team-lead model | Code |
| Number of workers | 1 (blocking) | 1 (concurrent) | 3–5 collaborators | Tens to hundreds |
| Inter-worker chat | No | No | Yes (mailbox) | No (stateless) |
| Determinism | Model-driven | Model-driven | Model-driven | Deterministic execution |
| Resumable | No | No | Limited | Yes (`/workflow resume` replays the checkpoint) |
| Best for | Side-quest during a turn | Specialist running in parallel | Debate / collaboration | Bulk fan-out |

## Troubleshooting

**"workflow: state.jsonl unavailable — proceeding without checkpoint"**
— `.thclaws/workflows/` can't be created or written. Check
permissions on `.thclaws/` in the project root.

**Script error: `ReferenceError: thclaws is not defined`** — you're
probably running a script outside `/workflow run`. The `thclaws.*`
global only exists inside the workflow sandbox.

**Workflow hangs after `⠋ wN  …` line** — that worker is taking a
while. Unless the call sets `budget: { time }`, subagent calls run
unbounded; Ctrl-C cancels the whole run. Add a per-worker `time`
budget in the script to cap individual workers.

**Re-author loop keeps producing the same script** — the model may be
ignoring your revision note. Try cancelling and re-running with a
sharper goal phrasing rather than relying on `r`-loops.
