// AUTOMATA docs: search, navigation drawer, copy buttons, and TOC highlighting.
// Generated into docs/site/docs/_static by scripts/build_site.py.
(() => {
  "use strict";

  // ---------- mobile navigation ----------
  const menu = document.querySelector(".menu");
  if (menu) {
    menu.addEventListener("click", () => {
      const open = document.body.classList.toggle("nav-open");
      menu.setAttribute("aria-expanded", String(open));
      menu.setAttribute("aria-label", open ? "Close navigation" : "Open navigation");
    });
  }
  const current = document.querySelector('.sidebar [aria-current="page"]');
  if (current) current.scrollIntoView({ block: "center" });

  // ---------- copy buttons ----------
  document.querySelectorAll(".codeblock .copy").forEach((button) => {
    button.addEventListener("click", async () => {
      const code = button.parentElement.querySelector("code").innerText;
      try {
        await navigator.clipboard.writeText(code.replace(/^\$ /gm, ""));
        button.textContent = "Copied";
      } catch {
        button.textContent = "Press Ctrl+C";
      }
      setTimeout(() => (button.textContent = "Copy"), 1500);
    });
  });

  // ---------- "on this page" highlighting ----------
  const tocLinks = [...document.querySelectorAll(".toc a")];
  if (tocLinks.length && "IntersectionObserver" in window) {
    const byId = new Map(tocLinks.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
    const visible = new Set();
    const observer = new IntersectionObserver((entries) => {
      for (const e of entries) e.isIntersecting ? visible.add(e.target.id) : visible.delete(e.target.id);
      const headings = [...byId.keys()].map((id) => document.getElementById(id)).filter(Boolean);
      const first = headings.find((h) => visible.has(h.id));
      if (!first) return;
      tocLinks.forEach((a) => a.classList.toggle("active", a === byId.get(first.id)));
    }, { rootMargin: "-60px 0px -65% 0px" });
    byId.forEach((_, id) => { const h = document.getElementById(id); if (h) observer.observe(h); });
  }

  // ---------- screenshots develop from a dither (docs/site/dither.js) ----------
  if (window.Dither) {
    document.querySelectorAll(".prose img").forEach((img) => {
      if (!img.closest(".banner")) window.Dither.develop(img, { hold: 200, duration: 800 });
    });
  }

  // ---------- search ----------
  const input = document.getElementById("search");
  const results = document.getElementById("results");
  if (!input || !results) return;
  const root = window.DOCS_ROOT || "";
  let selected = -1;

  const escape = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const terms = (q) => q.toLowerCase().split(/\s+/).filter(Boolean);
  const mark = (text, words) => {
    let out = escape(text);
    for (const w of words) {
      const re = new RegExp("(" + w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + ")", "ig");
      out = out.replace(re, "<mark>$1</mark>");
    }
    return out;
  };

  function search(query) {
    const words = terms(query);
    if (!words.length) return [];
    const hits = [];
    for (const page of window.SEARCH_INDEX || []) {
      const title = page.t.toLowerCase();
      const text = page.x.toLowerCase();
      let score = 0;
      let target = null;
      let ok = true;
      for (const w of words) {
        let s = 0;
        if (title.includes(w)) s += 12;
        const heading = page.h.find(([, h]) => h.toLowerCase().includes(w));
        if (heading) { s += 6; target = target || heading; }
        if (text.includes(w)) s += 1 + Math.min(4, text.split(w).length - 1) * 0.5;
        if (!s) { ok = false; break; }
        score += s;
      }
      if (!ok) continue;
      const at = text.indexOf(words[0]);
      const start = Math.max(0, at - 50);
      const snippet = (start ? "…" : "") + page.x.slice(start, start + 140).trim() + "…";
      hits.push({ page, score, target, snippet });
    }
    return hits.sort((a, b) => b.score - a.score).slice(0, 8);
  }

  function render() {
    const query = input.value.trim();
    selected = -1;
    if (!query) { results.hidden = true; input.setAttribute("aria-expanded", "false"); return; }
    const words = terms(query);
    const hits = search(query);
    results.innerHTML = hits.length
      ? hits.map(({ page, target, snippet }, i) => {
          const href = root + page.u + (target ? "#" + target[0] : "");
          const where = target ? target[1] : page.g;
          return `<li role="option"><a id="r${i}" href="${href}"><span class="r-title">${mark(page.t, words)}</span>` +
            `<span class="r-meta">${escape(where)}</span><span class="r-snip">${mark(snippet, words)}</span></a></li>`;
        }).join("")
      : `<li class="empty">No pages mention “${escape(query)}”. Try a shorter word.</li>`;
    results.hidden = false;
    input.setAttribute("aria-expanded", "true");
  }

  function move(delta) {
    const links = [...results.querySelectorAll("a")];
    if (!links.length) return;
    selected = (selected + delta + links.length) % links.length;
    links.forEach((a, i) => a.setAttribute("aria-selected", String(i === selected)));
    input.setAttribute("aria-activedescendant", links[selected].id);
    links[selected].scrollIntoView({ block: "nearest" });
  }

  input.addEventListener("input", render);
  input.addEventListener("focus", () => { if (input.value.trim()) render(); });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); move(1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(-1); }
    else if (e.key === "Enter") {
      const links = results.querySelectorAll("a");
      const link = links[selected >= 0 ? selected : 0];
      if (link) { e.preventDefault(); location.href = link.href; }
    } else if (e.key === "Escape") { input.value = ""; render(); input.blur(); }
  });
  document.addEventListener("click", (e) => {
    if (!e.target.closest(".search")) results.hidden = true;
  });
  document.addEventListener("keydown", (e) => {
    const typing = /INPUT|TEXTAREA|SELECT/.test(document.activeElement.tagName);
    if (e.key === "/" && !typing) { e.preventDefault(); input.focus(); input.select(); }
  });
})();
