# Chapter 27 — thClaws.cloud

thClaws.cloud is the catalog + hosted runtime for thClaws agents. It
turns the **folder-is-an-agent** model (Chapter 8) into something you
can browse, publish, install on someone else's machine, or rent a
hosted workspace for. From your desktop thClaws, the cloud feels like
git for AI agents — `cloud login` once, `cloud publish` from any
folder, `cloud get <slug>` to install someone else's work.

> **What this chapter covers (client side).** Browsing the catalog,
> publishing your own agents, getting agents into a local folder, and
> the new `agent.{name, description, uuid}` block in
> `settings.json`. The operator-side runbook for running your own
> catalog server lives in the dev plan ([`dev-plan/34`](../dev-plan/34-thclaws-cloud-control-plane.md))
> and the workspace-private `thclaws-cloud/` source tree.

## The folder-is-an-agent model — recap

Anywhere thClaws runs, an **AI agent is a folder**. Three files at the
root of that folder make it complete:

- `AGENTS.md` — the agent's instructions (system prompt + persona).
- `manifest.json` — catalog metadata (slug, license, icon, tags). Only
  needed to publish.
- `./.thclaws/` — local state (settings, KMS, sessions, memory).

When you `cd` into the folder and run thClaws, you're "running that
agent". When you publish, the catalog packages those files into a
tarball; when someone else does `cloud get <slug>`, they get the same
folder. The cloud is just a way to move folders between machines.

## Setting the catalog URL + a CLI token

Two things bind your desktop to a catalog server:

1. **Cloud URL** — `settings.json::cloud.url`. Defaults to the public
   instance (`https://thclaws.cloud`); point at `http://localhost` or
   your own self-hosted instance by overriding it.
2. **CLI token** — a `thc_…` string from the catalog dashboard, stored
   in the OS keychain (never in `settings.json`).

### Desktop GUI

Settings → **thClaws.cloud** has fields for both. Paste the URL, paste
the token from your dashboard's *Mint CLI token* button, hit Save —
the rest of this chapter's commands work immediately.

### CLI

```
$ thclaws cloud login                # prompts for the token interactively
$ thclaws cloud login --token thc_xxxxx
$ thclaws cloud status               # show resolved URL + whether a token is set
$ thclaws cloud logout               # forget the cached token
```

`cloud login` accepts a `--cloud-url URL` flag too if you want to
override the URL without editing `settings.json`.

## Browsing the catalog

From the REPL or Chat tab:

```
❯ /cloud status
thClaws.cloud — https://thclaws.cloud (token: ✓ stored)

❯ /cloud list
- hello-world           v0.1.0  Hello-world demo agent (jimmy)
- legal-doc-reviewer    v0.4.2  Reviews contracts paragraph-by-paragraph (acme)
- weekly-research       v1.0.0  Saturday-morning newsletter writer (rin)
...

❯ /cloud list --mine
- weekly-research       v1.0.0  Saturday-morning newsletter writer (you)
```

From the shell (same data, useful in scripts):

```
$ thclaws cloud list
$ thclaws cloud list --mine
```

Each row is a single agent in the catalog. The slug is what you pass
to `cloud get`.

## Installing an agent into a folder

```
❯ /cloud get hello-world
Downloading hello-world (v0.1.0) …
Extracted to /Users/jimmy/agents/hello-world/
  ✓ AGENTS.md
  ✓ manifest.json
  ✓ skills/greet.md
Done. cd hello-world && thclaws to run.
```

`/cloud get` (or `thclaws cloud get hello-world`) extracts the agent's
tarball into the current directory. The CLI form takes an optional
target dir:

```
$ thclaws cloud get hello-world ~/agents/hello-world
```

### The folder-safety check

`cloud get` refuses to overwrite a non-empty folder unless the folder
already holds the **same** agent (matched by UUID, see below) or you
pass `--force`. The check works like this:

| Target folder state | Behaviour |
|---|---|
| Empty | Fresh install. |
| Has `AGENTS.md` / `manifest.json` with a matching `agent.uuid` | Safe update — overwrites in place, preserves your `.thclaws/` session state. |
| Has `AGENTS.md` / `manifest.json` with a **mismatched** UUID | Abort with an error. The folder belongs to another agent. |
| Other random files (notes, scratch, etc.) | Abort unless `--force`. |

This is intentional — it prevents a typo from clobbering an
in-progress agent or someone else's work in the same directory.

## Publishing an agent

When you've built an agent in a folder and want it in the catalog:

```
$ cd ~/agents/my-research-bot
$ thclaws cloud publish              # uploads the cwd
$ thclaws cloud publish --dry-run    # preview the tarball contents, no upload
$ thclaws cloud publish ./other-dir  # publish a different folder
```

`publish` does three things:

1. **Tar + gzip** the folder. Secrets, sessions, KMS pages, and the
   `./.thclaws/` state directory are stripped automatically — you
   can re-publish daily without leaking conversation history.
2. **Upload** to the catalog using your CLI token.
3. **Stamp the agent identity back into `settings.json`** (see the
   next section).

If `manifest.json` is missing or invalid, publish aborts with a clear
error. Minimum required fields: `id`, `name`, `description`, `version`.

## Agent identity in `settings.json`

A new top-level `agent` block in `./.thclaws/settings.json` carries
this folder's catalog identity:

```json
{
  "agent": {
    "id": "my-research-bot",
    "name": "My Research Bot",
    "description": "Saturday-morning newsletter writer",
    "uuid": "1f9c1d70-3a26-43c4-9c40-1b1b6e3e3a01"
  }
}
```

- **id / name / description** — copied from `manifest.json` at publish
  time. Used by the catalog UI and by `cloud get`'s safety check.
- **uuid** — assigned by the catalog the **first** time you publish
  from this folder, written back into `settings.json`. Subsequent
  publishes hit the same catalog row (version bump). The UUID is what
  `cloud get` matches against to decide "is this folder the same
  agent?"

You normally don't edit this by hand. The GUI Settings → **Agent
identity** panel lets you tweak `name` / `description` (handy before
publishing — the description shows up in catalog listings) but
intentionally hides `uuid`.

### Forking a downloaded agent

If you `cloud get`-ed someone else's agent and want to fork it under
your own name:

```
$ thclaws cloud unbind        # clears settings.json::agent.uuid
$ # edit AGENTS.md, manifest.json — change `id` to something free
$ thclaws cloud publish        # gets a fresh UUID
```

Without `unbind`, the next publish would try to update the original
author's catalog row (and fail with a permission error — the catalog
gates publishes by author).

## Hosted workspaces (rent, don't install)

If you don't want to install agents on your laptop, the catalog also
runs them as **hosted workspaces** — one container per workspace, a
URL you open in any browser, a real chat UI backed by the same engine
you'd run locally.

From the catalog web UI:

1. Browse to an agent's detail page.
2. Click *Install on hosted*.
3. The catalog spins up a workspace, copies the agent's files in, and
   redirects you to the chat UI at `/u/<your-handle>/<slug>/`.

Hosted workspaces support both BYOK (paste your own provider keys
under *Settings → Hosted keys*) and the **thClaws.cloud gateway**
(pay-per-use proxy with credit billing — see below). The choice is a
radio toggle when you create the workspace.

## Pay-per-use gateway (alternative to BYOK)

For users who don't want to manage Anthropic / OpenAI / Gemini
accounts, thClaws.cloud offers a **gateway**: top up credits once,
then call any model through `gateway.thclaws.cloud/<provider>/...`
with a `gw_v1_…` token. The gateway forwards to upstream, meters the
response, and debits your balance.

To use the gateway from a **desktop** thClaws:

1. Mint a gateway access key in the catalog UI: **/gateway/keys** →
   *Mint new gateway key* → copy the `gw_v1_…` string.
2. Top up: **/credit** → pick a pack ($5 / $20 / $100). Bonus credit
   on the larger packs.
3. Configure thClaws to point at the gateway:
   ```bash
   export ANTHROPIC_API_KEY=gw_v1_…
   export ANTHROPIC_BASE_URL=https://thclaws.cloud/gateway/anthropic
   export OPENAI_API_KEY=gw_v1_…
   export OPENAI_BASE_URL=https://thclaws.cloud/gateway/openai/v1
   # …same for GEMINI_*, OPENROUTER_*
   ```
   (Or set the matching `*_API_KEY` / `*_BASE_URL` fields in the GUI
   Settings → Providers panel.)
4. Run thClaws normally. Calls go via the gateway; spend lands in
   **/credit/usage**.

For **hosted** workspaces, the gateway is auto-wired when you pick
*Gateway* at workspace-create time — the runner gets the env vars
injected, no copy-paste needed.

### Tier gating

Models are split into three tiers — `starter`, `pro`, `enterprise`.
Your account's `model_tier` (set in the catalog dashboard) controls
which models the gateway accepts. Starter accounts get Haiku /
gpt-4o-mini / Gemini Flash; calling Sonnet on starter returns a `403`
from the gateway with an upgrade link. Tiers are independent of
balance — having $100 in credit doesn't unlock enterprise models on
a starter account.

## Quick reference

| Command | Where | What it does |
|---|---|---|
| `thclaws cloud login [--token …]` | CLI | Store a CLI token in the keychain |
| `thclaws cloud logout` | CLI | Forget the cached token |
| `thclaws cloud status` | CLI / `/cloud status` | Show resolved URL + token state |
| `thclaws cloud list [--mine]` | CLI / `/cloud list` | Browse the catalog |
| `thclaws cloud get <slug> [<dir>] [--force]` | CLI / `/cloud get` | Install into a folder |
| `thclaws cloud publish [<dir>] [--dry-run]` | CLI | Upload from a folder |
| `thclaws cloud unbind` | CLI | Clear `agent.uuid` so the next publish creates a new catalog row |
| Settings → **thClaws.cloud** | GUI | URL + CLI token |
| Settings → **Agent identity** | GUI | Edit this folder's `agent.name` / `description` |
| `/credit` (web) | Catalog UI | Top up + view balance + browse pricing |
| `/gateway/keys` (web) | Catalog UI | Mint `gw_v1_…` access keys |
| `/credit/usage` (web) | Catalog UI | Per-call spend + per-workspace breakdown |

## What thClaws.cloud is not

A few things to set expectations:

- **Not a model host.** Catalog agents still call out to Anthropic /
  OpenAI / Gemini for inference — either via your own BYOK keys or
  via the cloud gateway as a billing proxy. thClaws.cloud doesn't
  train or serve LLMs itself.
- **Not session storage.** Conversation history stays in
  `./.thclaws/sessions/` on the machine that ran the agent. The cloud
  stores agent files, not conversations.
- **Not required.** Every chapter before this one works with no
  network at all. The cloud is additive — install thClaws, write
  `AGENTS.md`, and you have a useful agent without ever signing up.
