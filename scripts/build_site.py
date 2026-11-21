#!/usr/bin/env python3
"""Builds the documentation section of the AUTOMATA website.

Renders the Markdown wiki (``docs/wiki``) plus the engine docs into static
HTML under ``docs/site/docs/``, next to the landing page
(``docs/site/index.html``). Standard library only, so it runs anywhere:

    python scripts/build_site.py          # (re)build docs/site/docs
    python scripts/build_site.py --check  # CI: fail if the output is stale

Serve ``docs/`` (e.g. ``python -m http.server -d docs``) and open
``/site/``. Screenshots live in ``docs/assets`` and are shared with the
wiki, so the site must be served with ``docs/`` as its root (GitHub Pages
"from /docs" does exactly that).

The Markdown dialect is the subset the wiki uses: ATX headings, paragraphs,
nested lists, fenced code, tables, block quotes, rules, images, links,
inline code, bold and italics.
"""

from __future__ import annotations

import argparse
import html
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
SITE = DOCS / "site"
OUT = SITE / "docs"
REPO_URL = "https://github.com/x017/automata"
BRANCH = "main"

# Navigation: (group, [(source relative to docs/, label), ...]). Every page
# listed here is rendered; order drives the sidebar and prev / next links.
NAV: list[tuple[str, list[tuple[str, str]]]] = [
    ("Start", [
        ("wiki/README.md", "Overview"),
        ("wiki/getting-started.md", "Getting started"),
        ("wiki/tutorials/README.md", "All tutorials"),
    ]),
    ("Tutorials", [
        ("wiki/tutorials/first-scene.md", "Your first scene"),
        ("wiki/tutorials/coin-collector.md", "Coin collector"),
        ("wiki/tutorials/third-person-camera.md", "Third-person camera"),
        ("wiki/tutorials/camera-rail.md", "Camera rail & intro shot"),
        ("wiki/tutorials/lighting-and-mood.md", "Lighting & mood"),
        ("wiki/tutorials/oberon-to-python.md", "From Oberon to Python"),
        ("wiki/tutorials/fps.md", "A first-person shooter"),
        ("wiki/zero-to-hero.md", "Zero to hero (Python)"),
    ]),
    ("Oberon", [
        ("wiki/oberon/README.md", "Introduction"),
        ("wiki/oberon/interface.md", "The interface"),
        ("wiki/oberon/scripting.md", "Scripting"),
        ("wiki/oberon/cameras.md", "Cameras"),
        ("wiki/oberon/graphics.md", "Graphics"),
        ("wiki/oberon/code-editor.md", "The code editor"),
        ("wiki/oberon/models.md", "Models"),
        ("wiki/oberon/audio.md", "Audio"),
        ("wiki/oberon/prefabs-and-attributes.md", "Prefabs & attributes"),
        ("wiki/oberon/exporting.md", "Exporting your game"),
        ("wiki/oberon/shortcuts.md", "Keyboard shortcuts"),
    ]),
    ("Python guides", [
        ("wiki/guides/game-loop.md", "The game loop"),
        ("wiki/guides/scenes-and-entities.md", "Scenes & entities"),
        ("wiki/guides/rendering-and-lights.md", "Rendering & lights"),
        ("wiki/guides/cameras.md", "Cameras"),
        ("wiki/guides/input.md", "Input"),
        ("wiki/guides/physics.md", "Physics"),
        ("wiki/guides/events-and-resources.md", "Events & resources"),
        ("wiki/guides/oop-style.md", "OOP style"),
        ("wiki/guides/scene-files.md", "Scene files"),
        ("wiki/guides/project-structure.md", "Project structure"),
        ("wiki/guides/testing.md", "Testing"),
        ("wiki/guides/performance.md", "Performance"),
    ]),
    ("Reference", [
        ("wiki/api-reference.md", "API reference"),
        ("wiki/cookbook.md", "Cookbook"),
    ]),
    ("Engine", [
        ("architecture.md", "Architecture"),
        ("development.md", "Development"),
        ("api.md", "API design"),
        ("design/oberon.md", "Oberon design"),
    ]),
]


def output_path(source: str) -> str:
    """Site-relative output path (under docs/site/docs) for a source page."""
    if source.startswith("wiki/"):
        rel = source[len("wiki/"):]
    elif source.startswith("design/"):
        rel = "engine/" + source[len("design/"):].replace(".md", "-design.md")
    else:
        rel = "engine/" + source
    rel = re.sub(r"(^|/)README\.md$", r"\1index.md", rel)
    return rel[:-3] + ".html"


# --------------------------------------------------------------------------
# Inline Markdown
# --------------------------------------------------------------------------

def slugify(text: str) -> str:
    """GitHub-style heading anchor, so wiki links keep working."""
    text = re.sub(r"<[^>]+>", "", text).lower()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    return text.strip().replace(" ", "-")


@dataclass
class Context:
    """Per-page state: where links resolve from, and collected headings."""

    source: Path
    out: Path
    pages: dict[Path, str]
    headings: list[tuple[int, str, str]] = field(default_factory=list)
    slugs: dict[str, int] = field(default_factory=dict)
    title: str = ""

    def unique(self, slug: str) -> str:
        n = self.slugs.get(slug, 0)
        self.slugs[slug] = n + 1
        return slug if n == 0 else f"{slug}-{n}"

    def resolve(self, href: str) -> str:
        """Rewrites a Markdown link target for the site."""
        if re.match(r"^[a-z]+:", href) or href.startswith("#"):
            return href
        path, _, anchor = href.partition("#")
        target = (self.source.parent / path).resolve()
        suffix = f"#{anchor}" if anchor else ""
        if target in self.pages:
            dest = OUT / self.pages[target]
            return os.path.relpath(dest, self.out.parent).replace(os.sep, "/") + suffix
        try:
            rel_docs = target.relative_to(DOCS)
        except ValueError:
            rel_docs = None
        if rel_docs is not None and not str(rel_docs).endswith(".md") and target.exists():
            # Shared assets (screenshots): link into docs/ directly.
            return os.path.relpath(target, self.out.parent).replace(os.sep, "/") + suffix
        try:
            rel_repo = target.relative_to(ROOT)
        except ValueError:
            return href
        kind = "tree" if target.is_dir() else "blob"
        return f"{REPO_URL}/{kind}/{BRANCH}/{rel_repo.as_posix()}{suffix}"


def inline(text: str, ctx: Context) -> str:
    """Renders inline Markdown (already one logical paragraph)."""
    stash: list[str] = []

    def keep(fragment: str) -> str:
        stash.append(fragment)
        return f"\x00{len(stash) - 1}\x00"

    # Code spans first: their contents are literal.
    text = re.sub(
        r"(`+)(.+?)\1",
        lambda m: keep(f"<code>{html.escape(m.group(2).strip())}</code>"),
        text,
    )
    text = html.escape(text, quote=False)

    def image(m: re.Match[str]) -> str:
        alt, src = m.group(1), ctx.resolve(html.unescape(m.group(2)))
        return keep(
            f'<img src="{html.escape(src)}" alt="{alt}" loading="lazy" decoding="async">'
        )

    def link(m: re.Match[str]) -> str:
        label, href = m.group(1), ctx.resolve(html.unescape(m.group(2)))
        external = href.startswith("http")
        attrs = ' rel="noopener"' if external else ""
        return keep(f'<a href="{html.escape(href)}"{attrs}>{emphasis(label)}</a>')

    text = re.sub(r"!\[([^\]]*)\]\(([^)\s]+)\)", image, text)
    text = re.sub(r"\[((?:[^\[\]]|\x00\d+\x00)+)\]\(([^)\s]+)\)", link, text)
    text = re.sub(
        r"&lt;(https?://[^\s&]+)&gt;",
        lambda m: keep(f'<a href="{m.group(1)}" rel="noopener">{m.group(1)}</a>'),
        text,
    )
    text = emphasis(text)
    while "\x00" in text:
        text = re.sub(r"\x00(\d+)\x00", lambda m: stash[int(m.group(1))], text)
    return text


def emphasis(text: str) -> str:
    text = re.sub(r"\*\*(?=\S)(.+?)(?<=\S)\*\*", r"<strong>\1</strong>", text, flags=re.S)
    text = re.sub(r"~~(?=\S)(.+?)(?<=\S)~~", r"<del>\1</del>", text, flags=re.S)
    text = re.sub(r"(?<![\w*])\*(?=\S)([^*]+?)(?<=\S)\*(?![\w*])", r"<em>\1</em>", text)
    text = re.sub(r"(?<![\w_])_(?=\S)([^_]+?)(?<=\S)_(?![\w_])", r"<em>\1</em>", text)
    return text


# --------------------------------------------------------------------------
# Code highlighting
# --------------------------------------------------------------------------

PY_KEYWORDS = {
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
    "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield", "match", "case",
}
PY_CONSTANTS = {"True", "False", "None", "self", "cls"}
PY_BUILTINS = {
    "print", "len", "range", "min", "max", "abs", "sum", "any", "all", "int",
    "float", "str", "bool", "list", "dict", "set", "tuple", "zip", "enumerate",
    "sorted", "isinstance", "getattr", "setattr", "hasattr", "round", "super",
    "open", "type", "map", "filter", "reversed", "iter", "next",
}
RUST_KEYWORDS = {
    "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait",
    "for", "in", "if", "else", "match", "return", "self", "Self", "crate",
    "where", "loop", "while", "const", "static", "move", "ref", "as", "true",
    "false", "Some", "None", "Ok", "Err",
}

TOKEN = re.compile(
    r"""
    (?P<comment>\#[^\n]*|//[^\n]*)
  | (?P<string>[rbfRBF]{0,2}(?:\"\"\"[\s\S]*?\"\"\"|'''[\s\S]*?'''|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'))
  | (?P<number>\b\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?\b)
  | (?P<deco>@[A-Za-z_][\w.]*)
  | (?P<name>[A-Za-z_]\w*)
    """,
    re.VERBOSE,
)


def highlight(code: str, lang: str) -> str:
    lang = lang.lower()
    if lang in ("console", "sh", "shell", "bash"):
        out = []
        # Without any "$ " prompt the whole block is commands; with prompts,
        # the unprompted lines are program output.
        has_prompt = any(line.startswith("$ ") for line in code.split("\n"))
        for line in code.split("\n"):
            m = re.match(r"^(\$ )(.*)$", line)
            if m:
                out.append(f'<span class="tok-p">$ </span>{html.escape(m.group(2))}')
            elif line.lstrip().startswith("#"):
                out.append(f'<span class="tok-c">{html.escape(line)}</span>')
            elif has_prompt and line:
                out.append(f'<span class="tok-o">{html.escape(line)}</span>')
            else:
                out.append(html.escape(line))
        return "\n".join(out)
    if lang not in ("python", "py", "rust", "rs", "ron"):
        return html.escape(code)
    keywords = PY_KEYWORDS if lang in ("python", "py") else RUST_KEYWORDS
    out: list[str] = []
    pos = 0
    previous = ""
    for m in TOKEN.finditer(code):
        kind = m.lastgroup
        value = m.group()
        if kind == "comment":
            hash_comment = value.startswith("#")
            if hash_comment != (lang in ("python", "py")):
                continue  # not a comment in this language: leave it plain
        out.append(html.escape(code[pos:m.start()]))
        pos = m.end()
        css = {
            "comment": "c", "string": "s", "number": "n", "deco": "d",
        }.get(kind)
        if kind == "name":
            if value in keywords:
                css = "k"
            elif value in PY_CONSTANTS:
                css = "b"
            elif previous in ("def", "class", "fn", "struct", "enum"):
                css = "f"
            elif value in PY_BUILTINS:
                css = "b"
            elif lang == "ron" and value[:1].isupper():
                css = "t"
            previous = value
        out.append(f'<span class="tok-{css}">{html.escape(value)}</span>' if css else html.escape(value))
    out.append(html.escape(code[pos:]))
    return "".join(out)


# --------------------------------------------------------------------------
# Block Markdown
# --------------------------------------------------------------------------

LIST_ITEM = re.compile(r"^( {0,3})([-*+]|\d{1,9}[.)])( +|$)")
FENCE = re.compile(r"^ {0,3}(```+|~~~+)\s*([\w+-]*)")
HEADING = re.compile(r"^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$")
RULE = re.compile(r"^ {0,3}([-*_])(\s*\1){2,}\s*$")


def starts_block(line: str) -> bool:
    return bool(
        HEADING.match(line)
        or FENCE.match(line)
        or RULE.match(line)
        or line.lstrip().startswith(">")
        or LIST_ITEM.match(line)
        or line.lstrip().startswith("|")
    )


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def blocks(lines: list[str], ctx: Context, tight: bool = False) -> str:
    out: list[str] = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        if not line.strip():
            i += 1
            continue

        fence = FENCE.match(line)
        if fence:
            marker, lang = fence.group(1), fence.group(2)
            base = indent_of(line)
            i += 1
            body = []
            while i < n and not lines[i].strip().startswith(marker[0] * len(marker)):
                raw = lines[i]
                body.append(raw[min(base, indent_of(raw)):])
                i += 1
            i += 1
            code = "\n".join(body)
            label = f' data-lang="{lang}"' if lang and lang != "text" else ""
            out.append(
                f'<div class="codeblock"{label}><button class="copy" type="button" '
                f'aria-label="Copy code">Copy</button><pre><code>{highlight(code, lang)}'
                f"</code></pre></div>"
            )
            continue

        heading = HEADING.match(line)
        if heading:
            level = len(heading.group(1))
            text = heading.group(2)
            rendered = inline(text, ctx)
            plain = re.sub(r"<[^>]+>", "", rendered)
            if level == 1 and not ctx.title:
                ctx.title = html.unescape(plain)
                out.append(f"<h1>{rendered}</h1>")
            else:
                slug = ctx.unique(slugify(html.unescape(plain)))
                ctx.headings.append((level, slug, plain))
                out.append(
                    f'<h{level} id="{slug}">{rendered}'
                    f'<a class="anchor" href="#{slug}" aria-label="Link to this section">#</a>'
                    f"</h{level}>"
                )
            i += 1
            continue

        if RULE.match(line):
            out.append("<hr>")
            i += 1
            continue

        if line.lstrip().startswith(">"):
            quoted = []
            while i < n and lines[i].strip() and (
                lines[i].lstrip().startswith(">") or not starts_block(lines[i])
            ):
                quoted.append(re.sub(r"^\s*> ?", "", lines[i]))
                i += 1
            out.append(f'<aside class="note">{blocks(quoted, ctx)}</aside>')
            continue

        if line.lstrip().startswith("|") and i + 1 < n and re.match(
            r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$", lines[i + 1]
        ):
            out.append(table(lines, i, ctx))
            i += 2
            while i < n and lines[i].lstrip().startswith("|"):
                i += 1
            continue

        item = LIST_ITEM.match(line)
        if item:
            html_list, i = parse_list(lines, i, ctx)
            out.append(html_list)
            continue

        para = [line.strip()]
        i += 1
        while i < n and lines[i].strip() and not starts_block(lines[i]):
            para.append(lines[i].strip())
            i += 1
        rendered = inline("\n".join(para), ctx)
        out.append(rendered if tight else f"<p>{rendered}</p>")
    return "\n".join(out)


def split_row(row: str) -> list[str]:
    row = row.strip()
    if row.startswith("|"):
        row = row[1:]
    if row.endswith("|") and not row.endswith("\\|"):
        row = row[:-1]
    cells, current, code = [], "", False
    for ch in row:
        if ch == "`":
            code = not code
        if ch == "|" and not code:
            cells.append(current)
            current = ""
        else:
            current += ch
    cells.append(current)
    return [c.strip().replace("\\|", "|") for c in cells]


def table(lines: list[str], start: int, ctx: Context) -> str:
    head = split_row(lines[start])
    aligns = []
    for cell in split_row(lines[start + 1]):
        left, right = cell.startswith(":"), cell.endswith(":")
        aligns.append("center" if left and right else "right" if right else "")
    rows = []
    i = start + 2
    while i < len(lines) and lines[i].lstrip().startswith("|"):
        rows.append(split_row(lines[i]))
        i += 1

    def cell(tag: str, text: str, col: int) -> str:
        align = aligns[col] if col < len(aligns) and aligns[col] else ""
        style = f' style="text-align:{align}"' if align else ""
        return f"<{tag}{style}>{inline(text, ctx)}</{tag}>"

    thead = "".join(cell("th", t, c) for c, t in enumerate(head))
    tbody = "".join(
        "<tr>" + "".join(cell("td", t, c) for c, t in enumerate(r)) + "</tr>" for r in rows
    )
    return (
        f'<div class="table"><table><thead><tr>{thead}</tr></thead>'
        f"<tbody>{tbody}</tbody></table></div>"
    )


def parse_list(lines: list[str], i: int, ctx: Context) -> tuple[str, int]:
    first = LIST_ITEM.match(lines[i])
    assert first is not None
    base = len(first.group(1))
    ordered = first.group(2)[0].isdigit()
    start_number = int(first.group(2)[:-1]) if ordered else 1
    items: list[list[str]] = []
    loose = False
    n = len(lines)
    while i < n:
        m = LIST_ITEM.match(lines[i])
        if not m or len(m.group(1)) != base or m.group(2)[0].isdigit() != ordered:
            break
        content_indent = len(m.group(0)) if m.group(3) else len(m.group(0)) + 1
        body = [lines[i][len(m.group(0)):]]
        i += 1
        saw_blank = False
        while i < n:
            line = lines[i]
            if not line.strip():
                # A blank line continues the item only if indented content follows.
                j = i
                while j < n and not lines[j].strip():
                    j += 1
                if j < n and indent_of(lines[j]) >= content_indent:
                    body.extend([""] * (j - i))
                    saw_blank = True
                    i = j
                    continue
                if j < n and LIST_ITEM.match(lines[j]) and indent_of(lines[j]) == base:
                    loose = True
                break
            if indent_of(line) >= content_indent:
                body.append(line[content_indent:])
                i += 1
                continue
            sibling = LIST_ITEM.match(line)
            if sibling and len(sibling.group(1)) <= base:
                break
            if starts_block(line) or saw_blank:
                break
            body.append(line.strip())  # lazy continuation
            i += 1
        if saw_blank and any(not b.strip() for b in body):
            # Blank line between paragraphs inside an item makes it loose.
            stripped = [b for b in body if b.strip()]
            if stripped and not LIST_ITEM.match(stripped[-1]):
                loose = loose or _has_two_paragraphs(body)
        items.append(body)
    rendered = "".join(f"<li>{blocks(body, ctx, tight=not loose)}</li>" for body in items)
    if ordered:
        start = f' start="{start_number}"' if start_number != 1 else ""
        return f"<ol{start}>{rendered}</ol>", i
    return f"<ul>{rendered}</ul>", i


def _has_two_paragraphs(body: list[str]) -> bool:
    chunks, current = [], False
    for line in body:
        if line.strip():
            if not current:
                chunks.append(line)
            current = True
        else:
            current = False
    return len(chunks) > 1 and not all(LIST_ITEM.match(c) or FENCE.match(c) for c in chunks[1:])


# --------------------------------------------------------------------------
# Page template
# --------------------------------------------------------------------------

def nav_html(current: str, out_dir: Path) -> str:
    groups = []
    for group, pages in NAV:
        links = []
        for source, label in pages:
            href = os.path.relpath(OUT / output_path(source), out_dir).replace(os.sep, "/")
            here = ' aria-current="page"' if source == current else ""
            links.append(f'<li><a href="{href}"{here}>{html.escape(label)}</a></li>')
        groups.append(
            f'<div class="nav-group"><h2>{html.escape(group)}</h2><ul>{"".join(links)}</ul></div>'
        )
    return "".join(groups)


def toc_html(headings: list[tuple[int, str, str]]) -> str:
    items = [
        f'<li class="l{level}"><a href="#{slug}">{text}</a></li>'
        for level, slug, text in headings
        if level in (2, 3)
    ]
    if len(items) < 2:
        return ""
    return (
        '<nav class="toc" aria-label="On this page"><h2>On this page</h2>'
        f"<ul>{''.join(items)}</ul></nav>"
    )


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · AUTOMATA docs</title>
<meta name="description" content="{description}">
<meta name="theme-color" content="#000000">
<link rel="stylesheet" href="{static}/docs.css">
<script>window.DOCS_ROOT = "{root}";</script>
<script defer src="{static}/search-index.js"></script>
<script defer src="{site}dither.js"></script>
<script defer src="{static}/docs.js"></script>
</head>
<body>
<a class="skip" href="#content">Skip to content</a>
<header class="top">
  <button class="menu" type="button" aria-label="Open navigation" aria-expanded="false" aria-controls="sidebar">
    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 6h14M3 10h14M3 14h14" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
  </button>
  <a class="wordmark" href="{home}">AUTOMATA</a>
  <a class="section" href="{root}index.html">Docs</a>
  <div class="search" role="search">
    <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="9" cy="9" r="5.5" stroke="currentColor" stroke-width="1.6" fill="none"/><path d="M13.2 13.2 17 17" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
    <input id="search" type="search" placeholder="Search docs" autocomplete="off" spellcheck="false" aria-label="Search docs" aria-controls="results" aria-autocomplete="list">
    <kbd>/</kbd>
    <ul id="results" role="listbox" hidden></ul>
  </div>
  <a class="gh" href="{repo}" rel="noopener">GitHub</a>
</header>
<div class="layout">
  <nav class="sidebar" id="sidebar" aria-label="Documentation">{nav}</nav>
  <main id="content" class="article">
    <article class="prose">
{banner}{body}
    </article>
    <footer class="pager">{pager}</footer>
    <p class="edit"><a href="{edit}" rel="noopener">Edit this page on GitHub</a></p>
  </main>
  <aside class="aside">{toc}</aside>
</div>
</body>
</html>
"""


# Section home pages open with a dithered still of the scene renderer
# (exported from docs/site/dither.js; see docs/development.md).
BANNERS = {
    "wiki/README.md": "banner-docs.png",
    "wiki/tutorials/README.md": "banner-tutorials.png",
    "wiki/oberon/README.md": "banner-oberon.png",
}


def banner_html(src: str, out_dir: Path) -> str:
    name = BANNERS.get(src)
    if not name:
        return ""
    href = os.path.relpath(SITE / "img" / name, out_dir).replace(os.sep, "/")
    return (
        f'<figure class="banner" aria-hidden="true"><img src="{href}" width="760" height="240" alt=""></figure>\n'
    )


def pager_html(index: int, flat: list[tuple[str, str]], out_dir: Path) -> str:
    parts = []
    if index > 0:
        src, label = flat[index - 1]
        href = os.path.relpath(OUT / output_path(src), out_dir).replace(os.sep, "/")
        parts.append(f'<a class="prev" href="{href}"><small>Previous</small>{html.escape(label)}</a>')
    else:
        parts.append("<span></span>")
    if index + 1 < len(flat):
        src, label = flat[index + 1]
        href = os.path.relpath(OUT / output_path(src), out_dir).replace(os.sep, "/")
        parts.append(f'<a class="next" href="{href}"><small>Next</small>{html.escape(label)}</a>')
    return "".join(parts)


def plain_text(rendered: str) -> str:
    text = re.sub(r"<pre>[\s\S]*?</pre>", " ", rendered)
    text = re.sub(r"<[^>]+>", " ", text)
    return re.sub(r"\s+", " ", html.unescape(text)).strip()


def describe(body: str) -> str:
    """The first real sentence-bearing paragraph, for `<meta description>`."""
    for para in re.findall(r"<p>([\s\S]*?)</p>", body):
        text = re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", "", para))).strip()
        if len(text) >= 50 and text[0].isupper() and not text.endswith(":"):
            if len(text) > 200:
                text = text[:197].rsplit(" ", 1)[0] + "…"
            return text
    return ""


def build() -> dict[Path, str]:
    """Returns {output path: contents} for every generated file."""
    flat = [(src, label) for _, pages in NAV for src, label in pages]
    pages = {(DOCS / src).resolve(): output_path(src) for src, _ in flat}
    missing = sorted(
        str(p.relative_to(DOCS)) for p in DOCS.glob("wiki/**/*.md") if p.resolve() not in pages
    )
    if missing:
        raise SystemExit(f"wiki pages missing from NAV in {Path(__file__).name}: {', '.join(missing)}")
    files: dict[Path, str] = {}
    index = []
    for i, (src, label) in enumerate(flat):
        source = DOCS / src
        out = OUT / output_path(src)
        ctx = Context(source=source.resolve(), out=out, pages=pages)
        body = blocks(source.read_text(encoding="utf-8").split("\n"), ctx)
        out_dir = out.parent
        root = os.path.relpath(OUT, out_dir).replace(os.sep, "/")
        root = "" if root == "." else root + "/"
        text = plain_text(body)
        description = describe(body) or text
        files[out] = PAGE.format(
            title=html.escape(ctx.title or label),
            description=html.escape(description[:200], quote=True),
            static=f"{root}_static",
            site=os.path.relpath(SITE, out_dir).replace(os.sep, "/") + "/",
            banner=banner_html(src, out_dir),
            root=root,
            home=os.path.relpath(SITE / "index.html", out_dir).replace(os.sep, "/"),
            repo=REPO_URL,
            nav=nav_html(src, out_dir),
            body=body,
            pager=pager_html(i, flat, out_dir),
            edit=f"{REPO_URL}/edit/{BRANCH}/docs/{src}",
            toc=toc_html(ctx.headings),
        )
        group = next(g for g, ps in NAV if any(s == src for s, _ in ps))
        index.append({
            "t": ctx.title or label,
            "g": group,
            "u": output_path(src),
            "h": [[slug, re.sub(r"<[^>]+>", "", h)] for lvl, slug, h in ctx.headings if lvl <= 3],
            "x": text[:6000],
        })
    files[OUT / "_static" / "search-index.js"] = (
        "window.SEARCH_INDEX = " + json.dumps(index, ensure_ascii=False, separators=(",", ":")) + ";\n"
    )
    for name in ("docs.css", "docs.js"):
        files[OUT / "_static" / name] = (Path(__file__).parent / "site" / name).read_text(encoding="utf-8")
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="fail if docs/site/docs is stale")
    args = parser.parse_args()
    files = build()
    existing = {p for p in OUT.rglob("*") if p.is_file()} if OUT.exists() else set()
    if args.check:
        stale = [p for p, text in files.items() if not p.exists() or p.read_text(encoding="utf-8") != text]
        extra = existing - set(files)
        for p in stale:
            print(f"stale: {p.relative_to(ROOT)}")
        for p in sorted(extra):
            print(f"unexpected: {p.relative_to(ROOT)}")
        if stale or extra:
            print("docs site is out of date: run `python scripts/build_site.py`")
            return 1
        print(f"docs site up to date ({len(files)} files)")
        return 0
    for p in existing - set(files):
        p.unlink()
    for p, text in files.items():
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    print(f"wrote {len(files)} files to {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
