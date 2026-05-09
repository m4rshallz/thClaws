//! Write a research result to KMS as a timestamped raw note + update
//! the topic-level `_summary.md` consolidated view.
//!
//! Two-step write so consolidation can never lose data:
//! 1. **Raw note** — `<YYYY-MM-DD>-<query-slug>.md` with the full
//!    synthesized markdown verbatim. Timestamped filename means
//!    repeated `/research` calls on similar queries don't collide.
//! 2. **Summary** — `_summary.md` updated to reference the raw note
//!    (in M6.39.1, just appends a bullet; M6.39.2+ may add an LLM
//!    consolidation pass that merges raw notes into a coherent
//!    summary).
//!
//! KMS resolution: if `kms_target` is provided and exists, use it;
//! otherwise create a project-scoped KMS with that name. If no target
//! given, caller supplies an LLM-derived slug via
//! [`super::llm_calls::derive_topic_slug`].

use crate::error::Result;
use crate::kms::{self, KmsRef, KmsScope};
use std::path::PathBuf;

/// Result of a successful KMS write.
#[derive(Debug, Clone)]
pub struct WriteOutcome {
    /// Filename (no path) of the timestamped raw note inside the KMS.
    pub raw_note: String,
    /// Absolute path on disk where it landed (useful for log lines).
    pub raw_note_path: PathBuf,
    /// Name of the KMS the note went to (handy when caller passed
    /// `None` and we auto-derived).
    pub kms_name: String,
}

/// Build the raw note filename from today + a query slug. Format:
/// `2026-05-09-obon-festival.md`. Timestamp prefix sorts naturally so
/// `ls` shows newest at the bottom.
pub fn make_raw_filename(today: &str, query_slug: &str) -> String {
    format!("{today}-{}", query_slug)
}

/// Resolve or create the target KMS. Always project-scoped — research
/// is workspace-specific by default. M6.39.2 may add `--user-kms` to
/// flip to user scope.
fn resolve_or_create_kms(name: &str) -> Result<KmsRef> {
    if let Some(k) = kms::resolve(name) {
        return Ok(k);
    }
    kms::create(name, KmsScope::Project)
}

/// M6.39.6: public alias of `resolve_or_create_kms` so the pipeline
/// can ensure the KMS exists before kicking off parallel page-synth
/// futures (each one will later write into it).
pub fn resolve_or_create(name: &str) -> Result<KmsRef> {
    resolve_or_create_kms(name)
}

/// Write the synthesized result + update summary. Returns the relative
/// raw-note filename (caller surfaces as `JobView::result_page`).
pub fn write(
    kms_name: &str,
    query: &str,
    query_slug: &str,
    today: &str,
    synthesized: &str,
) -> Result<WriteOutcome> {
    let kref = resolve_or_create_kms(kms_name)?;
    let filename = make_raw_filename(today, query_slug);
    let body = build_raw_note_body(query, today, synthesized);
    let raw_path = kms::write_page(&kref, &filename, &body)?;
    update_summary(&kref, today, query, &filename)?;
    Ok(WriteOutcome {
        raw_note: format!("{filename}.md"),
        raw_note_path: raw_path,
        kms_name: kref.name.clone(),
    })
}

/// Compose the raw note body with frontmatter that the KMS schema-aware
/// lint (M6.37) can validate against. The `type: research` discriminator
/// lets users filter `KmsSearch` to research notes only.
fn build_raw_note_body(query: &str, today: &str, synthesized: &str) -> String {
    // M6.39.5: body leads with synthesized content directly. Pre-fix
    // the body opened with `# Research: <query>` H1 + `**Query:**` +
    // `**Date:**` lines that just restated frontmatter. The KMS
    // auto-index pulls each page's "summary" from
    // `first_meaningful_line(body)`, which strips Markdown markers
    // and returned just `Research: <query>` — useless for the LLM
    // trying to decide whether the page is relevant.
    //
    // With this layout, the synthesize prompt's required 1-2
    // sentence abstract becomes the first meaningful line, and the
    // index summary actually describes what's inside.
    let _ = query;
    format!(
        "---\n\
         title: \"Research: {}\"\n\
         type: research\n\
         query: \"{}\"\n\
         created: {today}\n\
         updated: {today}\n\
         ---\n\n\
         {}\n",
        escape_yaml_string(query),
        escape_yaml_string(query),
        synthesized.trim()
    )
}

/// Append a one-line bullet to `_summary.md` so users have an index of
/// research entries in this KMS without opening each file. M6.39.1
/// keeps it dumb (chronological bullet list); future revs can add an
/// LLM consolidation pass over the bullets.
fn update_summary(kref: &KmsRef, today: &str, query: &str, raw_filename: &str) -> Result<()> {
    let line = format!(
        "- {today} — [{}]({}.md) — {}\n",
        truncate_for_summary(query, 80),
        raw_filename,
        raw_filename
    );
    let summary_name = "_summary";
    // KMS pages live under `<root>/pages/<stem>.md` — checking
    // `root.join("_summary.md")` (an earlier version of this check)
    // always returned "missing" and triggered a destructive
    // `write_page` (create-or-replace) on the second call, wiping
    // the first call's bullet. The real existence check is in
    // pages_dir.
    let summary_path = kref.pages_dir().join(format!("{summary_name}.md"));
    if !summary_path.exists() {
        let body = format!(
            "---\n\
             title: \"Research summary\"\n\
             type: research-summary\n\
             updated: {today}\n\
             ---\n\n\
             # Research summary\n\n\
             Auto-maintained index of research notes in this knowledge base. \
             Each line links to the timestamped raw note generated by `/research`.\n\n\
             {line}"
        );
        kms::write_page(kref, summary_name, &body)?;
    } else {
        kms::append_to_page(kref, summary_name, &line)?;
    }
    Ok(())
}

fn truncate_for_summary(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max - 1 {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// YAML string escaping — research queries can contain quotes,
/// backslashes, control chars. Keep it simple: escape `"` and `\` and
/// drop ASCII control chars (newlines etc).
fn escape_yaml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Bridge into the system-time → date-string conversion used by the
/// rest of KMS. Callers in tests pass an explicit date; production
/// uses [`crate::usage::today_str`].
pub fn today_str() -> String {
    crate::usage::today_str()
}

/// M6.39.5: persist a fetched source page into the KMS's `sources/`
/// directory for offline provenance. Called once per cited source
/// (those whose `[N]` index appears in the synthesized markdown) so
/// the KMS contains both the synthesized note AND the raw inputs the
/// LLM saw at synthesis time.
///
/// Filename is a deterministic slug of the URL — same URL across
/// different research runs maps to the same file (the latest fetch
/// wins). Frontmatter carries the original URL + citation index +
/// query so a user browsing `sources/` can trace which research run
/// produced each cached page.
pub fn write_source(
    kms_name: &str,
    query: &str,
    today: &str,
    index: u32,
    title: &str,
    url: &str,
    body: &str,
) -> Result<std::path::PathBuf> {
    let kref = crate::kms::resolve(kms_name).ok_or_else(|| {
        crate::error::Error::Tool(format!(
            "KMS '{kms_name}' not found — research-source write needs an existing KMS"
        ))
    })?;
    let dir = kref.root.join("sources");
    std::fs::create_dir_all(&dir).map_err(|e| {
        crate::error::Error::Tool(format!("create sources dir {}: {e}", dir.display()))
    })?;
    let filename = url_to_filename(url);
    let path = dir.join(format!("{filename}.md"));
    let body_str = format!(
        "---\n\
         type: research-source\n\
         title: \"{}\"\n\
         url: \"{}\"\n\
         fetched_for: \"{}\"\n\
         citation_index: {index}\n\
         fetched_at: {today}\n\
         ---\n\n\
         # {}\n\n\
         **Source:** [{}]({})\n\n\
         {}\n",
        escape_yaml_string(title),
        escape_yaml_string(url),
        escape_yaml_string(query),
        title,
        url,
        url,
        body.trim()
    );
    std::fs::write(&path, body_str)
        .map_err(|e| crate::error::Error::Tool(format!("write {}: {e}", path.display())))?;
    Ok(path)
}

/// URL → filesystem-safe slug. Strip protocol, lowercase, replace
/// non-alphanumerics with `-`, collapse runs, trim, cap at 80 chars.
/// `https://en.wikipedia.org/wiki/Obon` → `en-wikipedia-org-wiki-obon`.
fn url_to_filename(url: &str) -> String {
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let mut slug = String::with_capacity(stripped.len());
    let mut prev_dash = true;
    for c in stripped.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "source".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// M6.39.6: write one page from a multi-page research run. Filename
/// shape `<run-prefix>__<page-slug>.md` so multiple pages from the
/// same research run sort together in the index, and pages from
/// different runs don't collide. Frontmatter discriminator
/// `type: research-page` (sibling of `type: research` for legacy
/// single-page output).
pub fn write_research_page(
    kms_name: &str,
    run_prefix: &str,
    page_slug: &str,
    page_title: &str,
    page_topic: &str,
    query: &str,
    today: &str,
    body: &str,
) -> Result<std::path::PathBuf> {
    let kref = resolve_or_create_kms(kms_name)?;
    let filename = format!("{run_prefix}__{page_slug}");
    let composed = format!(
        "---\n\
         title: \"{}\"\n\
         type: research-page\n\
         page_slug: {}\n\
         run: {}\n\
         query: \"{}\"\n\
         topic: \"{}\"\n\
         created: {today}\n\
         updated: {today}\n\
         ---\n\n\
         {}\n",
        escape_yaml_string(page_title),
        page_slug,
        run_prefix,
        escape_yaml_string(query),
        escape_yaml_string(page_topic),
        body.trim()
    );
    crate::kms::write_page(&kref, &filename, &composed)
}

/// M6.39.6: rewrite Obsidian-style `[[slug]]` and `[[slug|display]]`
/// cross-links so they target the actual on-disk filename. Pages
/// are written with `<run-prefix>__<slug>` filenames; a bare
/// `[[karpathy]]` would not resolve because the file is
/// `2026-05-09-llm-wiki__karpathy.md`. Rewriter replaces the slug
/// portion with the prefixed form so Obsidian (and any markdown
/// renderer that respects wikilinks) can resolve it.
///
/// Display text (`[[slug|display]]`) is preserved verbatim so the
/// reader sees a clean human label.
///
/// Slugs not present in `known_slugs` are left untouched —
/// `[[some-other-page]]` may legitimately reference an existing
/// wiki entry from a previous research run, so we don't mangle
/// those.
pub fn rewrite_cross_links(body: &str, known_slugs: &[&str], run_prefix: &str) -> String {
    let known: std::collections::HashSet<&str> = known_slugs.iter().copied().collect();
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len() + 64);
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Find the closing `]]`.
            if let Some(end_rel) = body[i + 2..].find("]]") {
                let inner_start = i + 2;
                let inner_end = inner_start + end_rel;
                let inner = &body[inner_start..inner_end];
                // Length sanity — Obsidian wikilinks are short. >120
                // chars is almost certainly not a wikilink.
                if inner.len() <= 120 && !inner.contains('\n') {
                    let (slug_part, display_part) = match inner.split_once('|') {
                        Some((s, d)) => (s.trim(), Some(d.trim())),
                        None => (inner.trim(), None),
                    };
                    if known.contains(slug_part) {
                        let new_slug = format!("{run_prefix}__{slug_part}");
                        out.push_str("[[");
                        out.push_str(&new_slug);
                        if let Some(d) = display_part {
                            out.push('|');
                            out.push_str(d);
                        }
                        out.push_str("]]");
                        i = inner_end + 2;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// M6.39.6: append a "Run on `<date>`" section to `_summary.md`
/// listing every page produced by this research run. Replaces the
/// old single-bullet append from the legacy single-page write.
/// Each line links to the run's pages so the user can drill in
/// from the summary.
pub fn append_run_section(
    kms_name: &str,
    run_prefix: &str,
    today: &str,
    query: &str,
    pages: &[(String, String, String)], // (slug, title, topic)
) -> Result<()> {
    let kref = crate::kms::resolve(kms_name).ok_or_else(|| {
        crate::error::Error::Tool(format!(
            "KMS '{kms_name}' not found — append_run_section needs an existing KMS"
        ))
    })?;
    let mut section = format!("\n## {today} — {}\n\n", truncate_for_summary(query, 80));
    for (slug, title, topic) in pages {
        let filename = format!("{run_prefix}__{slug}");
        let topic_brief = if topic.trim().is_empty() {
            String::new()
        } else {
            format!(" — {}", truncate_for_summary(topic, 100))
        };
        section.push_str(&format!("- [[{filename}|{title}]]{topic_brief}\n"));
    }
    let summary_name = "_summary";
    let summary_path = kref.pages_dir().join(format!("{summary_name}.md"));
    if !summary_path.exists() {
        let body = format!(
            "---\n\
             title: \"Research summary\"\n\
             type: research-summary\n\
             updated: {today}\n\
             ---\n\n\
             # Research summary\n\n\
             Auto-maintained index of research notes in this knowledge base. \
             Each section corresponds to one `/research` run; pages within a \
             run are cross-linked.\n\
             {section}"
        );
        crate::kms::write_page(&kref, summary_name, &body)?;
    } else {
        crate::kms::append_to_page(&kref, summary_name, &section)?;
    }
    Ok(())
}

/// M6.39.5: parse citation indices `[N]` out of synthesized markdown.
/// Returns a set so duplicates collapse. Used by the pipeline to
/// decide which fetched sources to persist into `sources/`.
///
/// Matches `[1]`, `[12]`, `[1,3,7]`, `[1, 3]` — comma-separated
/// indices in a single bracket are common in academic-style synth
/// output. Non-numeric brackets (`[ref]`, `[link](url)`) are
/// ignored. False positives are cheap (one extra source file);
/// false negatives leak provenance, so the parser leans permissive.
pub fn parse_citation_indices(markdown: &str) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    let bytes = markdown.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        // Find matching `]`. Bail if missing or content longer than 30
        // chars (excludes `[link text with stuff](url)` and other
        // non-citation brackets).
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b']' {
            end += 1;
        }
        if end >= bytes.len() || end - start == 0 || end - start > 30 {
            i += 1;
            continue;
        }
        let inner = &markdown[start..end];
        // Inner must be only digits + commas + spaces — anything else
        // disqualifies (markdown link text, image alt, etc.).
        if !inner
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == ' ')
        {
            i += 1;
            continue;
        }
        for tok in inner.split(',') {
            if let Ok(n) = tok.trim().parse::<u32>() {
                if n > 0 {
                    out.insert(n);
                }
            }
        }
        i = end + 1;
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use crate::research::test_helpers::scoped_home;

    #[test]
    fn make_raw_filename_format() {
        assert_eq!(
            make_raw_filename("2026-05-09", "obon-festival"),
            "2026-05-09-obon-festival"
        );
    }

    #[test]
    fn yaml_escape_handles_quotes_and_backslashes() {
        assert_eq!(escape_yaml_string(r#"hello "world""#), r#"hello \"world\""#);
        assert_eq!(escape_yaml_string(r"path\to\file"), r"path\\to\\file");
    }

    #[test]
    fn yaml_escape_strips_control_chars() {
        let s = "line one\nline two\tindented";
        let escaped = escape_yaml_string(s);
        assert!(!escaped.contains('\n'));
        assert!(!escaped.contains('\t'));
        assert!(escaped.contains("line one line two indented"));
    }

    #[test]
    fn truncate_for_summary_preserves_short_strings() {
        assert_eq!(truncate_for_summary("short", 50), "short");
    }

    #[test]
    fn truncate_for_summary_adds_ellipsis_when_long() {
        let s = "a".repeat(100);
        let t = truncate_for_summary(&s, 20);
        assert_eq!(t.chars().count(), 20);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn truncate_for_summary_handles_unicode() {
        let s = "ค้นหาข่าวล่าสุดเกี่ยวกับเทคโนโลยี";
        let t = truncate_for_summary(s, 10);
        assert_eq!(t.chars().count(), 10);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn build_raw_note_includes_required_frontmatter_keys() {
        let body = build_raw_note_body("test query", "2026-05-09", "Some synthesized content");
        assert!(body.starts_with("---\n"));
        assert!(body.contains("title: \"Research: test query\""));
        assert!(body.contains("type: research"));
        assert!(body.contains("query: \"test query\""));
        assert!(body.contains("created: 2026-05-09"));
        assert!(body.contains("updated: 2026-05-09"));
        assert!(body.contains("Some synthesized content"));
    }

    /// M6.39.5: pin the post-fix body shape so the synthesized
    /// abstract becomes the first non-frontmatter line. Pre-fix the
    /// body led with `# Research: <query>` H1 + `**Query:**` /
    /// `**Date:**` lines that just restated frontmatter — kms's
    /// `first_meaningful_line` indexer would pull `Research: <query>`
    /// as the page summary, useless for the LLM picking pages to read.
    #[test]
    fn build_raw_note_body_starts_with_synthesized_content_after_frontmatter() {
        let synthesized = "OBON is a Japanese summer festival honoring \
                           ancestors [1][3].\n\n## History\n\nMore content here.";
        let body = build_raw_note_body("what is OBON", "2026-05-09", synthesized);
        // Body MUST NOT contain the old preamble — those were the
        // exact things crowding out the abstract.
        assert!(
            !body.contains("# Research: what is OBON"),
            "body should not lead with H1 — frontmatter title covers it"
        );
        assert!(
            !body.contains("**Query:**"),
            "body should not restate Query — frontmatter has `query:`"
        );
        assert!(
            !body.contains("**Date:**"),
            "body should not restate Date — frontmatter has `created:`/`updated:`"
        );
        // The first non-frontmatter line must be the abstract prose.
        let after_fm = body.splitn(2, "---\n\n").nth(1).unwrap();
        let first_line = after_fm.lines().next().unwrap();
        assert!(
            first_line.starts_with("OBON is a Japanese"),
            "first non-frontmatter line should be the abstract, got: {first_line}"
        );
    }

    /// Integration test: write to a temp KMS and verify both files land
    /// with the right shape. Uses the same `scoped_home` helper pattern
    /// as `kms::tests`.
    #[test]
    fn write_creates_raw_and_summary_in_fresh_kms() {
        let _g = scoped_home();
        let outcome = write(
            "research-test-kms",
            "what is OBON",
            "obon-festival",
            "2026-05-09",
            "Obon is a Japanese festival [1].\n\n## Sources\n[1] https://example.com\n",
        )
        .unwrap();
        assert_eq!(outcome.raw_note, "2026-05-09-obon-festival.md");
        assert_eq!(outcome.kms_name, "research-test-kms");
        assert!(outcome.raw_note_path.exists());
        let raw = std::fs::read_to_string(&outcome.raw_note_path).unwrap();
        assert!(raw.contains("type: research"));
        assert!(raw.contains("https://example.com"));

        // Summary also exists with the expected bullet entry.
        let kref = kms::resolve("research-test-kms").unwrap();
        let summary_path = kref.pages_dir().join("_summary.md");
        assert!(summary_path.exists(), "summary should be created");
        let summary = std::fs::read_to_string(&summary_path).unwrap();
        assert!(summary.contains("type: research-summary"));
        assert!(summary.contains("2026-05-09 — [what is OBON]"));
        assert!(summary.contains("(2026-05-09-obon-festival.md)"));
    }

    /// End-to-end: write a research note whose body leads with an
    /// abstract, then verify the KMS auto-index picks up that abstract
    /// as the page summary (not "Research: ..." or any meta line).
    /// This is the exact flow that fixes the "LLM ignores KMS"
    /// observation from /system inspection — an informative summary
    /// signals page relevance to the model deciding which page to
    /// KmsRead.
    #[test]
    fn write_index_summary_uses_abstract_not_title() {
        let _g = scoped_home();
        let synthesized = "OBON is a Japanese festival honoring ancestors \
                           observed 13–16 August [1][3].\n\n## History\nmore.";
        write(
            "research-test-index",
            "what is OBON",
            "what-is-obon",
            "2026-05-09",
            synthesized,
        )
        .unwrap();
        let kref = kms::resolve("research-test-index").unwrap();
        let index = std::fs::read_to_string(&kref.index_path()).unwrap();
        // The auto-index should have a bullet for the page using the
        // abstract's first line as the summary, NOT "Research: ..."
        // or "Query: ...".
        assert!(
            index.contains("OBON is a Japanese festival"),
            "index should contain the abstract as page summary, got:\n{index}"
        );
        assert!(
            !index.contains("Research: what is OBON"),
            "index should NOT use the H1 title as summary (that's the pre-fix bug):\n{index}"
        );
    }

    // ── rewrite_cross_links ────────────────────────────────────────

    #[test]
    fn rewrite_cross_links_simple_slug() {
        let body = "See also [[karpathy]] for more.";
        let out = rewrite_cross_links(body, &["karpathy"], "2026-05-09-llm-wiki");
        assert_eq!(out, "See also [[2026-05-09-llm-wiki__karpathy]] for more.");
    }

    #[test]
    fn rewrite_cross_links_with_display_text() {
        let body = "See also [[karpathy|Andrej Karpathy]] for more.";
        let out = rewrite_cross_links(body, &["karpathy"], "2026-05-09-llm-wiki");
        assert_eq!(
            out,
            "See also [[2026-05-09-llm-wiki__karpathy|Andrej Karpathy]] for more."
        );
    }

    #[test]
    fn rewrite_cross_links_leaves_unknown_slugs_alone() {
        // `[[some-other-page]]` may reference an existing entry
        // from a prior research run — don't mangle it.
        let body = "See [[unknown-slug]] and [[karpathy]].";
        let out = rewrite_cross_links(body, &["karpathy"], "run-2");
        assert!(out.contains("[[unknown-slug]]"));
        assert!(out.contains("[[run-2__karpathy]]"));
    }

    #[test]
    fn rewrite_cross_links_handles_multiple_in_same_body() {
        let body = "[[a]] then [[b|Bee]] then [[a|Aye]].";
        let out = rewrite_cross_links(body, &["a", "b"], "r");
        assert!(out.contains("[[r__a]]"));
        assert!(out.contains("[[r__b|Bee]]"));
        assert!(out.contains("[[r__a|Aye]]"));
    }

    #[test]
    fn rewrite_cross_links_preserves_non_link_brackets() {
        // `[1]` is a citation, not a wikilink. Single brackets
        // shouldn't be touched.
        let body = "Cite [1] and link [[karpathy]] in same line.";
        let out = rewrite_cross_links(body, &["karpathy"], "r");
        assert!(out.contains("[1]"));
        assert!(out.contains("[[r__karpathy]]"));
    }

    #[test]
    fn rewrite_cross_links_skips_overlong_inner() {
        // > 120 chars between [[ and ]] = not a wikilink
        let inner: String = "x".repeat(150);
        let body = format!("[[{inner}]]");
        let out = rewrite_cross_links(&body, &["x"], "r");
        // Should pass through unchanged (no slug match anyway, but
        // the length guard must fire first).
        assert_eq!(out, body);
    }

    #[test]
    fn rewrite_cross_links_skips_multiline_inner() {
        let body = "[[karpathy\nbroken]]";
        let out = rewrite_cross_links(body, &["karpathy"], "r");
        // Multiline = not a wikilink.
        assert_eq!(out, body);
    }

    // ── append_run_section + write_research_page integration ──────

    #[test]
    fn write_research_page_creates_page_with_run_prefix() {
        let _g = scoped_home();
        let _ = crate::kms::create("multi-page-test", crate::kms::KmsScope::Project).unwrap();
        let path = write_research_page(
            "multi-page-test",
            "2026-05-09-test-query",
            "karpathy",
            "Andrej Karpathy",
            "Karpathy's role as proponent",
            "what is OBON",
            "2026-05-09",
            "Andrej Karpathy is a researcher [1].\n\n## Background\n\nMore here.",
        )
        .unwrap();
        assert!(path
            .to_str()
            .unwrap()
            .ends_with("2026-05-09-test-query__karpathy.md"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("type: research-page"));
        assert!(body.contains("page_slug: karpathy"));
        assert!(body.contains("run: 2026-05-09-test-query"));
        assert!(body.contains("Andrej Karpathy is a researcher"));
    }

    #[test]
    fn append_run_section_creates_new_summary_with_section() {
        let _g = scoped_home();
        let _ = crate::kms::create("run-summary-test", crate::kms::KmsScope::Project).unwrap();
        let pages = vec![
            (
                "concept".to_string(),
                "Concept".to_string(),
                "Core idea".to_string(),
            ),
            (
                "karpathy".to_string(),
                "Karpathy".to_string(),
                "Person page".to_string(),
            ),
        ];
        append_run_section(
            "run-summary-test",
            "2026-05-09-llm-wiki",
            "2026-05-09",
            "what is llm-wiki",
            &pages,
        )
        .unwrap();
        let kref = crate::kms::resolve("run-summary-test").unwrap();
        let summary = std::fs::read_to_string(kref.pages_dir().join("_summary.md")).unwrap();
        assert!(summary.contains("type: research-summary"));
        assert!(summary.contains("## 2026-05-09 — what is llm-wiki"));
        assert!(summary.contains("[[2026-05-09-llm-wiki__concept|Concept]]"));
        assert!(summary.contains("[[2026-05-09-llm-wiki__karpathy|Karpathy]]"));
    }

    #[test]
    fn append_run_section_appends_to_existing_summary() {
        let _g = scoped_home();
        let _ = crate::kms::create("run-append-test", crate::kms::KmsScope::Project).unwrap();
        append_run_section(
            "run-append-test",
            "2026-05-09-first",
            "2026-05-09",
            "first query",
            &[("a".into(), "A".into(), "first topic".into())],
        )
        .unwrap();
        append_run_section(
            "run-append-test",
            "2026-05-10-second",
            "2026-05-10",
            "second query",
            &[("b".into(), "B".into(), "second topic".into())],
        )
        .unwrap();
        let kref = crate::kms::resolve("run-append-test").unwrap();
        let summary = std::fs::read_to_string(kref.pages_dir().join("_summary.md")).unwrap();
        assert!(summary.contains("first query"));
        assert!(summary.contains("second query"));
        assert!(summary.contains("[[2026-05-09-first__a|A]]"));
        assert!(summary.contains("[[2026-05-10-second__b|B]]"));
    }

    // ── parse_citation_indices ─────────────────────────────────────

    #[test]
    fn parse_citation_indices_basic() {
        let md = "Result here [1] and another claim [2].";
        let got = parse_citation_indices(md);
        assert_eq!(got.len(), 2);
        assert!(got.contains(&1));
        assert!(got.contains(&2));
    }

    #[test]
    fn parse_citation_indices_handles_comma_lists() {
        let md = "Multi-cite [1,3,5] and [2, 4].";
        let got = parse_citation_indices(md);
        assert_eq!(got.len(), 5);
        for n in [1, 2, 3, 4, 5] {
            assert!(got.contains(&n), "expected {n} in {:?}", got);
        }
    }

    #[test]
    fn parse_citation_indices_dedupes() {
        let md = "Same source twice [3] and again [3] elsewhere.";
        let got = parse_citation_indices(md);
        assert_eq!(got.len(), 1);
        assert!(got.contains(&3));
    }

    #[test]
    fn parse_citation_indices_ignores_markdown_links() {
        // Standard markdown link — `[text](url)` shouldn't trigger
        // even though brackets contain digits.
        let md = "See [the docs](https://example.com) for [3] more.";
        let got = parse_citation_indices(md);
        assert_eq!(got.len(), 1);
        assert!(got.contains(&3));
    }

    #[test]
    fn parse_citation_indices_ignores_non_numeric() {
        let md = "Plain text [abc] and [ref-foo] but [7] is real.";
        let got = parse_citation_indices(md);
        assert_eq!(got.len(), 1);
        assert!(got.contains(&7));
    }

    #[test]
    fn parse_citation_indices_empty_on_no_citations() {
        let md = "Just prose, no bracketed numbers anywhere.";
        assert!(parse_citation_indices(md).is_empty());
    }

    #[test]
    fn parse_citation_indices_skips_zero() {
        // [0] is meaningless as a citation index (citations are 1-based).
        let md = "Should ignore [0] and pick up [1].";
        let got = parse_citation_indices(md);
        assert_eq!(got.len(), 1);
        assert!(got.contains(&1));
    }

    #[test]
    fn parse_citation_indices_long_brackets_ignored() {
        // > 30 chars between brackets = probably not a citation.
        let md = "[this is a very very very very long bracket content with 1, 2, 3]";
        assert!(parse_citation_indices(md).is_empty());
    }

    // ── url_to_filename ────────────────────────────────────────────

    #[test]
    fn url_to_filename_strips_protocol() {
        assert_eq!(
            url_to_filename("https://en.wikipedia.org/wiki/Obon"),
            "en-wikipedia-org-wiki-obon"
        );
        assert_eq!(url_to_filename("http://example.com/foo"), "example-com-foo");
    }

    #[test]
    fn url_to_filename_handles_query_string() {
        assert_eq!(
            url_to_filename("https://example.com/path?q=1&r=2"),
            "example-com-path-q-1-r-2"
        );
    }

    #[test]
    fn url_to_filename_caps_at_80_chars() {
        let long = format!("https://example.com/{}", "a".repeat(200));
        let slug = url_to_filename(&long);
        assert!(slug.len() <= 80);
    }

    #[test]
    fn url_to_filename_falls_back_for_empty() {
        assert_eq!(url_to_filename(""), "source");
        assert_eq!(url_to_filename("---"), "source");
    }

    // ── write_source integration ───────────────────────────────────

    #[test]
    fn write_source_creates_file_in_sources_dir() {
        let _g = scoped_home();
        // Need an existing KMS to write into.
        let _ = crate::kms::create("test-kms", crate::kms::KmsScope::Project).unwrap();
        let path = write_source(
            "test-kms",
            "what is OBON",
            "2026-05-09",
            3,
            "Obon Festival - Wikipedia",
            "https://en.wikipedia.org/wiki/Obon",
            "Body content of the wikipedia page about Obon...",
        )
        .unwrap();
        assert!(path.exists());
        // File lives under <kms-root>/sources/, not pages/.
        assert!(path.to_str().unwrap().contains("/sources/"));
        assert!(!path.to_str().unwrap().contains("/pages/"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("---\n"));
        assert!(body.contains("type: research-source"));
        assert!(body.contains("citation_index: 3"));
        assert!(body.contains("https://en.wikipedia.org/wiki/Obon"));
        assert!(body.contains("Body content of the wikipedia"));
    }

    #[test]
    fn write_source_errors_on_unknown_kms() {
        let _g = scoped_home();
        let err = write_source(
            "no-such-kms",
            "q",
            "2026-05-09",
            1,
            "title",
            "https://x.example",
            "body",
        )
        .unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn second_write_appends_to_existing_summary() {
        let _g = scoped_home();
        write(
            "research-multi",
            "first query",
            "first-query",
            "2026-05-09",
            "first answer",
        )
        .unwrap();
        write(
            "research-multi",
            "second query",
            "second-query",
            "2026-05-10",
            "second answer",
        )
        .unwrap();
        let kref = kms::resolve("research-multi").unwrap();
        let summary = std::fs::read_to_string(kref.pages_dir().join("_summary.md")).unwrap();
        assert!(summary.contains("first query"));
        assert!(summary.contains("second query"));
    }
}
