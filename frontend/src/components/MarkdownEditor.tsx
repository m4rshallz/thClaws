import { useEditor, EditorContent, Node } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import { marked } from "marked";
import TurndownService from "turndown";
import { useEffect, useRef } from "react";

interface Props {
  source: string;
  onChange: (markdown: string) => void;
}

// Markdown ↔ HTML round-trip via marked + turndown. TipTap works in
// HTML natively; `tiptap-markdown` does not parse markdown on
// `setContent`, which is why clicking Edit on a `.md` used to render
// the raw `#` / `-` markers as plain paragraphs. `async: false`
// forces `marked.parse` to return a string synchronously so TipTap
// never sees `[object Promise]`.
marked.setOptions({ gfm: true, breaks: false, async: false });

// ── Preserve raw HTML comments through the round-trip ────────────────
// ProseMirror's DOM parser silently DROPS comment nodes (`<!-- … -->`),
// so wrapper markers like `<!-- img:foo -->` were lost on every save. We
// pre-transform each comment into a `<div data-html-comment>` placeholder
// that survives DOM parsing, hold it as an atom node (shown as a muted
// chip via CSS), and turn it back into a real comment on serialize.
const HtmlComment = Node.create({
  name: "htmlComment",
  group: "block",
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      text: {
        default: "",
        parseHTML: (el: HTMLElement) => el.getAttribute("data-html-comment") || "",
        renderHTML: (attrs: Record<string, unknown>) => ({
          "data-html-comment": String(attrs.text ?? ""),
        }),
      },
    };
  },
  parseHTML() {
    return [{ tag: "div[data-html-comment]" }];
  },
  renderHTML({ HTMLAttributes }: { HTMLAttributes: Record<string, unknown> }) {
    return ["div", { ...HTMLAttributes, class: "md-html-comment" }];
  },
});

function escapeAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// Turn marked's emitted `<!-- … -->` into placeholder divs. Comments
// inside code blocks are already entity-escaped by marked (`&lt;!--`),
// so this only matches real, block-level comments.
function commentsToPlaceholders(html: string): string {
  return html.replace(
    /<!--([\s\S]*?)-->/g,
    (_m, inner: string) => `<div data-html-comment="${escapeAttr(inner)}"></div>`,
  );
}

const turndownService = new TurndownService({
  headingStyle: "atx",
  bulletListMarker: "-",
  codeBlockStyle: "fenced",
  emDelimiter: "_",
});
// Placeholder div → real HTML comment. (Images use turndown's built-in
// rule → `![alt](src)`.)
turndownService.addRule("htmlComment", {
  filter: (node) =>
    node.nodeName === "DIV" && node.getAttribute("data-html-comment") !== null,
  replacement: (_content, node) =>
    "<!--" + ((node as HTMLElement).getAttribute("data-html-comment") || "") + "-->",
});

export function MarkdownEditor({ source, onChange }: Props) {
  // Track the last markdown we emitted so an echoed `source` prop
  // doesn't reset the editor and jump the caret on every keystroke.
  const lastEmittedRef = useRef<string | null>(null);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({}),
      Image.configure({ inline: false, allowBase64: true }),
      HtmlComment,
    ],
    content: "",
    onUpdate: ({ editor }) => {
      const html = editor.getHTML();
      const md = turndownService.turndown(html).trim() + "\n";
      lastEmittedRef.current = md;
      onChange(md);
    },
    editorProps: {
      attributes: {
        // `tiptap-compact` + the inline <style> below mirror
        // InstructionsEditorModal's styling. The `prose` Tailwind
        // typography plugin isn't installed, and Tailwind 4 preflight
        // strips heading sizes + list markers — so without these rules
        // headings and bullets render as plain paragraphs.
        class:
          "tiptap-compact max-w-none focus:outline-none px-4 py-3",
        spellcheck: "false",
      },
    },
  });

  useEffect(() => {
    if (!editor) return;
    if (lastEmittedRef.current === source) return;
    const parsed = marked.parse(source);
    const html = commentsToPlaceholders(typeof parsed === "string" ? parsed : "");
    queueMicrotask(() => {
      editor.commands.setContent(html, {
        emitUpdate: false,
        parseOptions: { preserveWhitespace: false },
      });
      lastEmittedRef.current = source;
    });
  }, [source, editor]);

  return (
    <div
      className="flex-1 min-h-0 overflow-auto rounded border"
      style={{
        background: "var(--bg-secondary)",
        borderColor: "var(--border)",
      }}
    >
      <style>{`
        .tiptap-compact { font-size: 13px; line-height: 1.55; }
        .tiptap-compact p { font-size: 13px; margin-top: 0.35em; margin-bottom: 0.35em; }
        .tiptap-compact h1 { font-size: 1.4rem; font-weight: 600; margin-top: 0.7em; margin-bottom: 0.35em; }
        .tiptap-compact h2 { font-size: 1.2rem; font-weight: 600; margin-top: 0.6em; margin-bottom: 0.3em; }
        .tiptap-compact h3 { font-size: 1.05rem; font-weight: 600; margin-top: 0.55em; margin-bottom: 0.25em; }
        .tiptap-compact h4, .tiptap-compact h5, .tiptap-compact h6 { font-size: 0.95rem; font-weight: 600; margin-top: 0.5em; margin-bottom: 0.2em; }
        .tiptap-compact ul {
          list-style: disc;
          list-style-position: outside;
          margin-top: 0.3em;
          margin-bottom: 0.3em;
          padding-left: 1.5em;
        }
        .tiptap-compact ol {
          list-style: decimal;
          list-style-position: outside;
          margin-top: 0.3em;
          margin-bottom: 0.3em;
          padding-left: 1.5em;
        }
        .tiptap-compact ul ul { list-style: circle; }
        .tiptap-compact ul ul ul { list-style: square; }
        .tiptap-compact li {
          font-size: 13px;
          margin-top: 0.15em;
          margin-bottom: 0.15em;
          display: list-item;
        }
        .tiptap-compact li > p { margin: 0; }
        .tiptap-compact code { font-size: 12px; padding: 0.1em 0.3em; border-radius: 3px; background: rgba(127,127,127,0.15); }
        .tiptap-compact pre { font-size: 12px; padding: 0.6em 0.8em; border-radius: 4px; background: rgba(127,127,127,0.12); overflow-x: auto; }
        .tiptap-compact pre code { background: transparent; padding: 0; }
        .tiptap-compact blockquote { font-size: 13px; margin: 0.4em 0; padding-left: 0.8em; border-left: 3px solid rgba(127,127,127,0.35); color: var(--text-secondary); }
        .tiptap-compact a { color: var(--accent, #61afef); text-decoration: underline; }
        .tiptap-compact strong { font-weight: 600; }
        .tiptap-compact em { font-style: italic; }
        .tiptap-compact hr { border: none; border-top: 1px solid var(--border); margin: 0.8em 0; }
        .tiptap-compact img { max-width: 100%; height: auto; border-radius: 4px; margin: 0.4em 0; }
        .tiptap-compact .md-html-comment {
          font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
          font-size: 11px;
          color: var(--text-secondary);
          opacity: 0.65;
          margin: 0.25em 0;
          white-space: pre-wrap;
          user-select: none;
        }
        .tiptap-compact .md-html-comment::before { content: "<!--" attr(data-html-comment) "-->"; }
        .tiptap-compact .md-html-comment.ProseMirror-selectednode { outline: 2px solid var(--accent, #61afef); border-radius: 3px; opacity: 1; }
      `}</style>
      <EditorContent editor={editor} className="h-full" />
    </div>
  );
}
