/* AUTOMATA landing page.
 *
 * A tiny hidden-line 3D renderer on a 2D canvas, drawn to look like Oberon's
 * scene view: a fading grid, red and blue axis lines, flat-shaded solids with
 * bright edges, and a move gizmo on the selected entity. Two scenes use it:
 * the hero (a player hopping across platforms, collecting coins) and the
 * coin.py playground, whose numbers are draggable and feed the same logic as
 * the real script. Plain script, no dependencies.
 */
"use strict";
(() => {
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const COL = {
    text: [242, 240, 235], body: [207, 204, 197], dim: [161, 159, 152], faint: [124, 122, 117],
    x: [230, 92, 92], y: [120, 204, 112], z: [92, 144, 230], coin: [242, 193, 78], grid: [118, 126, 150],
  };
  const rgba = (c, a) => `rgba(${c[0]},${c[1]},${c[2]},${a.toFixed(3)})`;
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const lerp = (a, b, t) => a + (b - a) * t;
  const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };
  const LIGHT = norm([0.45, 1, 0.3]);
  const NEAR = 0.15;

  // ---------- meshes (unit size, outward counter-clockwise faces) ----------
  const BOX = {
    verts: Array.from({ length: 8 }, (_, i) => [(i & 1) - 0.5, ((i >> 1) & 1) - 0.5, ((i >> 2) & 1) - 0.5]),
    faces: [[0, 4, 6, 2], [1, 3, 7, 5], [0, 1, 5, 4], [2, 6, 7, 3], [0, 2, 3, 1], [4, 5, 7, 6]],
  };
  function disc(n) {
    const verts = [];
    for (const z of [0.5, -0.5]) {
      for (let i = 0; i < n; i++) {
        const a = (i / n) * Math.PI * 2 + Math.PI / n;
        verts.push([Math.cos(a) * 0.5, Math.sin(a) * 0.5, z]);
      }
    }
    const faces = [Array.from({ length: n }, (_, i) => i), Array.from({ length: n }, (_, i) => 2 * n - 1 - i)];
    for (let i = 0; i < n; i++) faces.push([i, n + i, n + ((i + 1) % n), (i + 1) % n]);
    return { verts, faces };
  }
  const COIN = disc(8);

  // ---------- view: camera, projection, drawing ----------
  class View {
    constructor(canvas, cam) {
      this.canvas = canvas;
      this.g = canvas.getContext("2d");
      Object.assign(this, { yaw: 0.6, pitch: 0.4, dist: 14, target: [0, 1, 0], fov: 48, cx: 0.5, cy: 0.5 }, cam);
      this.resize();
      new ResizeObserver(() => { this.resize(); this.onResize?.(); }).observe(canvas);
    }
    resize() {
      const r = this.canvas.getBoundingClientRect();
      this.dpr = Math.min(2, devicePixelRatio || 1);
      this.w = Math.max(1, r.width);
      this.h = Math.max(1, r.height);
      this.canvas.width = Math.round(this.w * this.dpr);
      this.canvas.height = Math.round(this.h * this.dpr);
    }
    begin() {
      const { g } = this;
      g.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
      g.fillStyle = "#000";
      g.fillRect(0, 0, this.w, this.h);
      const cp = Math.cos(this.pitch), t = this.target;
      this.eye = [t[0] + this.dist * cp * Math.sin(this.yaw), t[1] + this.dist * Math.sin(this.pitch), t[2] + this.dist * cp * Math.cos(this.yaw)];
      this.f = norm(sub(t, this.eye));
      this.r = norm(cross(this.f, [0, 1, 0]));
      this.u = cross(this.r, this.f);
      this.focal = (Math.min(this.h, this.w * 0.9) / 2) / Math.tan((this.fov * Math.PI) / 360);
      this.faces = [];
    }
    cam(p) { const d = sub(p, this.eye); return [dot(d, this.r), dot(d, this.u), dot(d, this.f)]; }
    scr(c) { const s = this.focal / c[2]; return [this.w * this.cx + c[0] * s, this.h * this.cy - c[1] * s]; }
    point(p) { const c = this.cam(p); return c[2] < NEAR ? null : this.scr(c); }
    seg(a, b) {
      let A = this.cam(a), B = this.cam(b);
      if (A[2] < NEAR && B[2] < NEAR) return null;
      if (A[2] < NEAR) { const k = (NEAR - A[2]) / (B[2] - A[2]); A = A.map((v, i) => lerp(v, B[i], k)); }
      else if (B[2] < NEAR) { const k = (NEAR - B[2]) / (A[2] - B[2]); B = B.map((v, i) => lerp(v, A[i], k)); }
      return [this.scr(A), this.scr(B)];
    }

    /** Ground grid on y = 0, fading out with distance from the origin. */
    grid(n, fade, strength = 0.42) {
      const buckets = Array.from({ length: 7 }, () => new Path2D());
      const axes = { x: new Path2D(), z: new Path2D() };
      const steps = 2 * n;
      for (let i = -n; i <= n; i++) {
        for (let k = 0; k < steps; k++) {
          const s0 = -n + k, s1 = s0 + 1, mid = s0 + 0.5;
          for (const dir of [0, 1]) {
            const a = dir ? [s0, 0, i] : [i, 0, s0];
            const b = dir ? [s1, 0, i] : [i, 0, s1];
            const alpha = 1 - Math.hypot(dir ? mid : i, dir ? i : mid) / fade;
            if (alpha <= 0.02) continue;
            const sg = this.seg(a, b);
            if (!sg) continue;
            const path = i === 0 ? (dir ? axes.x : axes.z) : buckets[Math.min(6, Math.floor(alpha * 7))];
            path.moveTo(sg[0][0], sg[0][1]);
            path.lineTo(sg[1][0], sg[1][1]);
          }
        }
      }
      const { g } = this;
      g.lineWidth = 1;
      buckets.forEach((p, i) => { g.strokeStyle = rgba(COL.grid, ((i + 0.5) / 7) * strength); g.stroke(p); });
      g.lineWidth = 1.3;
      g.strokeStyle = rgba(COL.x, 0.75); g.stroke(axes.x);
      g.strokeStyle = rgba(COL.z, 0.75); g.stroke(axes.z);
    }

    /** Queue a mesh instance; faces from every instance are depth-sorted in end(). */
    mesh(m, { pos, scale = [1, 1, 1], yaw = 0, color = COL.dim, alpha = 1, glow = 0 }) {
      const cy = Math.cos(yaw), sy = Math.sin(yaw);
      const world = m.verts.map((v) => {
        const x = v[0] * scale[0], y = v[1] * scale[1], z = v[2] * scale[2];
        return [x * cy + z * sy + pos[0], y + pos[1], -x * sy + z * cy + pos[2]];
      });
      for (const f of m.faces) {
        const p0 = world[f[0]], p1 = world[f[1]], p2 = world[f[2]];
        const n = norm(cross(sub(p1, p0), sub(p2, p0)));
        if (dot(n, sub(this.eye, p0)) <= 0) continue;
        const cams = f.map((i) => this.cam(world[i]));
        if (cams.some((c) => c[2] < NEAR)) continue;
        const depth = cams.reduce((s, c) => s + c[2], 0) / cams.length;
        const light = 0.35 + 0.65 * Math.max(0, dot(n, LIGHT));
        this.faces.push({ pts: cams.map((c) => this.scr(c)), depth, color, alpha, light, glow });
      }
    }

    end(fog = [10, 34]) {
      const { g } = this;
      this.faces.sort((a, b) => b.depth - a.depth);
      g.lineJoin = "round";
      for (const f of this.faces) {
        const fade = clamp(1.1 - (f.depth - fog[0]) / (fog[1] - fog[0]), 0.28, 1) * f.alpha;
        g.beginPath();
        f.pts.forEach((p, i) => (i ? g.lineTo(p[0], p[1]) : g.moveTo(p[0], p[1])));
        g.closePath();
        const k = (0.05 + f.glow * 0.1) * f.light;
        g.fillStyle = `rgba(${Math.round(4 + f.color[0] * k)},${Math.round(4 + f.color[1] * k)},${Math.round(4 + f.color[2] * k)},${Math.min(1, f.alpha * 1.4).toFixed(3)})`;
        g.fill();
        g.lineWidth = 1.15;
        g.strokeStyle = rgba(f.color, fade);
        g.stroke();
      }
    }

    /** Oberon-style move gizmo: three axis arrows that always draw on top. */
    gizmo(at, len = 1.1) {
      const o = this.point(at);
      if (!o) return;
      const { g } = this;
      for (const [axis, c] of [[[len, 0, 0], COL.x], [[0, len, 0], COL.y], [[0, 0, len], COL.z]]) {
        const e = this.point([at[0] + axis[0], at[1] + axis[1], at[2] + axis[2]]);
        if (!e) continue;
        const dx = e[0] - o[0], dy = e[1] - o[1], l = Math.hypot(dx, dy) || 1;
        const ux = dx / l, uy = dy / l;
        g.strokeStyle = rgba(c, 1);
        g.lineWidth = 2;
        g.beginPath(); g.moveTo(o[0], o[1]); g.lineTo(e[0], e[1]); g.stroke();
        g.fillStyle = rgba(c, 1);
        g.beginPath();
        g.moveTo(e[0] + ux * 9, e[1] + uy * 9);
        g.lineTo(e[0] - uy * 4.5, e[1] + ux * 4.5);
        g.lineTo(e[0] + uy * 4.5, e[1] - ux * 4.5);
        g.fill();
      }
      g.fillStyle = rgba(COL.text, 1);
      g.beginPath(); g.arc(o[0], o[1], 3, 0, Math.PI * 2); g.fill();
    }

    ring(c, radius, color, alpha, dashed = true) {
      const { g } = this;
      g.beginPath();
      let started = false;
      for (let i = 0; i <= 48; i++) {
        const a = (i / 48) * Math.PI * 2;
        const p = this.point([c[0] + Math.cos(a) * radius, 0.01, c[2] + Math.sin(a) * radius]);
        if (!p) { started = false; continue; }
        if (started) g.lineTo(p[0], p[1]); else { g.moveTo(p[0], p[1]); started = true; }
      }
      g.setLineDash(dashed ? [5, 5] : []);
      g.lineWidth = 1.2;
      g.strokeStyle = rgba(color, alpha);
      g.stroke();
      g.setLineDash([]);
    }

    line(a, b, color, alpha, dash = []) {
      const s = this.seg(a, b);
      if (!s) return;
      const { g } = this;
      g.setLineDash(dash);
      g.lineWidth = 1;
      g.strokeStyle = rgba(color, alpha);
      g.beginPath(); g.moveTo(s[0][0], s[0][1]); g.lineTo(s[1][0], s[1][1]); g.stroke();
      g.setLineDash([]);
    }

    label(p, text, color, alpha = 1, dx = 10, dy = -10, size = 12) {
      const s = this.point(p);
      if (!s) return;
      const { g } = this;
      g.font = `600 ${size}px Bricolage, system-ui, sans-serif`;
      g.fillStyle = rgba(color, alpha);
      g.fillText(text, s[0] + dx, s[1] + dy);
    }
  }

  // ---------- a run loop that sleeps while off screen ----------
  function animate(canvas, frame) {
    let visible = true, raf = 0, last = performance.now(), t = 0;
    const tick = (now) => {
      raf = 0;
      t += Math.min(0.05, (now - last) / 1000);
      last = now;
      frame(t);
      if (visible && !reduced) raf = requestAnimationFrame(tick);
    };
    const kick = () => { if (!raf) { last = performance.now(); raf = requestAnimationFrame(tick); } };
    new IntersectionObserver(([e]) => { visible = e.isIntersecting; if (visible) kick(); }).observe(canvas);
    return { redraw: kick, time: () => t };
  }

  // Orbit by dragging; vertical page scrolling keeps working on touch.
  function orbit(view, canvas, onMove) {
    let drag = null;
    canvas.addEventListener("pointerdown", (e) => {
      drag = { x: e.clientX, y: e.clientY, yaw: view.yaw, pitch: view.pitch };
      canvas.setPointerCapture(e.pointerId);
      canvas.classList.add("dragging");
    });
    canvas.addEventListener("pointermove", (e) => {
      if (!drag) return;
      view.yaw = drag.yaw - (e.clientX - drag.x) * 0.006;
      view.pitch = clamp(drag.pitch + (e.clientY - drag.y) * 0.004, 0.12, 1.15);
      onMove();
    });
    const up = () => { drag = null; canvas.classList.remove("dragging"); };
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("pointercancel", up);
    return () => drag !== null;
  }

  // ---------- line numbers for the code panel (before the numbers get their listeners) ----------
  const code = document.getElementById("code");
  if (code) code.innerHTML = code.innerHTML.split("\n").map((l) => `<span class="line">${l || " "}</span>`).join("");

  // ---------- hero: a lap of a tiny platformer ----------
  (function hero() {
    const canvas = document.getElementById("world");
    if (!canvas) return;
    const view = new View(canvas, { yaw: 0.55, pitch: 0.38, dist: 21, target: [0.6, 1.2, 0], fov: 44 });
    const layout = () => {
      const wide = view.w > 760;
      view.cx = wide ? 0.68 : 0.5;
      view.cy = wide ? 0.47 : 0.5;
      view.dist = wide ? 21 : 17;
    };
    layout();
    view.onResize = () => { layout(); loop.redraw(); };

    const platforms = [
      { pos: [-4.5, 0.5, 0.5], scale: [2.4, 1, 2.4] },
      { pos: [-1, 1, -1.5], scale: [2.2, 2, 2.2] },
      { pos: [2.6, 0.75, 0.2], scale: [2.4, 1.5, 2.4] },
      { pos: [5.8, 1.5, -1.8], scale: [2, 3, 2] },
      { pos: [-7.5, 2, -5], scale: [1, 4, 1] },
      { pos: [0.5, 0.15, -6.2], scale: [15, 0.3, 0.5] },
    ];
    // Hops between platform tops, then a walk back along the front.
    const P = (x, y, z) => [x, y, z];
    const plan = [
      ["rest", P(-7, 0, 3), 0.5],
      ["hop", P(-4.5, 1, 0.5)], ["rest", null, 0.22],
      ["hop", P(-1, 2, -1.5)], ["rest", null, 0.22],
      ["hop", P(2.6, 1.5, 0.2)], ["rest", null, 0.22],
      ["hop", P(5.8, 3, -1.8)], ["rest", null, 0.4],
      ["hop", P(8, 0, 1.4)], ["rest", null, 0.2],
      ["walk", P(7, 0, 3.3)], ["walk", P(-7, 0, 3)],
    ];
    const segs = [];
    let at = plan[0][1], clock = 0;
    for (const [kind, to, d] of plan) {
      if (kind === "rest") { segs.push({ kind, a: at, b: at, t0: clock, dur: d }); clock += d; continue; }
      const dist = Math.hypot(to[0] - at[0], to[2] - at[2]);
      const dur = kind === "hop" ? 0.5 + dist * 0.07 + Math.abs(to[1] - at[1]) * 0.05 : dist / 3.4;
      segs.push({ kind, a: at, b: to, t0: clock, dur });
      clock += dur;
      at = to;
    }
    const LAP = clock;

    function playerAt(lt) {
      const s = segs.find((g) => lt < g.t0 + g.dur) || segs[segs.length - 1];
      const k = clamp((lt - s.t0) / s.dur, 0, 1);
      const pos = s.a.map((v, i) => lerp(v, s.b[i], k));
      let squash = 1;
      if (s.kind === "hop") {
        const h = 1.1 + Math.max(0, s.b[1] - s.a[1]);
        pos[1] += h * 4 * k * (1 - k);
        squash = 1 + 0.08 * Math.sin(k * Math.PI);
      } else if (s.kind === "rest") {
        squash = 1 - 0.12 * Math.sin(k * Math.PI);
      } else {
        pos[1] += Math.abs(Math.sin(lt * 9)) * 0.06;
      }
      return { pos, squash, s, k };
    }

    // Coins sit at each hop's apex and along the walk back.
    const coins = [];
    for (const s of segs) {
      if (s.kind === "hop" && s.b[1] > 0) coins.push({ tc: s.t0 + s.dur / 2 });
    }
    const walk = segs[segs.length - 1];
    for (const k of [0.3, 0.65]) coins.push({ tc: walk.t0 + walk.dur * k });
    for (const c of coins) { const p = playerAt(c.tc).pos; c.home = [p[0], p[1] + 0.75, p[2]]; }

    let heading = 0;
    const dragging = orbit(view, canvas, () => loop.redraw());
    let drift = 0;
    const loop = animate(canvas, (t) => {
      if (!dragging()) { drift += (reduced ? 0 : 1) * 0.016; view.yaw += Math.sin(drift * 0.35) * 0.0009; }
      const lt = reduced ? 3.1 : t % LAP;
      view.begin();
      view.grid(13, 13.5);
      for (const p of platforms) view.mesh(BOX, { ...p, color: COL.dim });

      const pl = playerAt(lt);
      const dir = sub(pl.s.b, pl.s.a);
      if (Math.hypot(dir[0], dir[2]) > 0.01) heading = Math.atan2(dir[0], dir[2]);
      const ph = 1.4 * pl.squash;
      view.mesh(BOX, { pos: [pl.pos[0], pl.pos[1] + ph / 2, pl.pos[2]], scale: [0.72 / Math.sqrt(pl.squash), ph, 0.72 / Math.sqrt(pl.squash)], yaw: heading, color: COL.text, glow: 1 });

      const pops = [];
      for (const c of coins) {
        const age = lt - c.tc;
        const spawn = clamp(lt / 0.4, 0, 1);
        if (age > 0.45) { if (age < 1) pops.push({ c, age }); continue; }
        const up = age > 0 ? age * 2.2 : 0;
        const s = (age > 0 ? 1 - age / 0.45 : 1) * spawn;
        if (s <= 0.01) continue;
        const bob = Math.sin(t * 3 + c.home[0]) * 0.12;
        view.mesh(COIN, { pos: [c.home[0], c.home[1] + bob + up, c.home[2]], scale: [0.62 * s, 0.62 * s, 0.12 * s], yaw: t * 2.5 + c.home[0], color: COL.coin, glow: 0.6 });
        if (age > 0) pops.push({ c, age });
      }
      view.end([12, 34]);

      // Drop line from the player to the ground, like a placement guide.
      const foot = [pl.pos[0], 0, pl.pos[2]];
      if (pl.pos[1] > 0.05) view.line(foot, pl.pos, COL.faint, 0.6, [3, 4]);
      view.gizmo([pl.pos[0], pl.pos[1] + ph / 2, pl.pos[2]], 1.05);
      view.label([pl.pos[0], pl.pos[1] + ph + 0.1, pl.pos[2]], "Player", COL.text, 0.85, 12, -4);
      for (const { c, age } of pops) {
        view.label(c.home, "+1", COL.coin, clamp(1 - age, 0, 1), -6, -14 - age * 34, 15);
      }
    });
  })();

  // ---------- coin.py playground ----------
  (function playground() {
    const canvas = document.getElementById("coins");
    if (!canvas) return;
    const DEFAULTS = { radius: 1.1, amp: 0.15, freq: 3, spin: 2.5 };
    const P = { ...DEFAULTS };
    const view = new View(canvas, { yaw: 0.42, pitch: 0.56, dist: 9.2, target: [0, 0.45, 0.8], fov: 42, cy: 0.42 });
    const coins = [[-2.6, 0.95, 0.45], [0, 0.95, 1.0], [2.6, 0.95, 1.55]].map((home) => ({ home, collected: false, at: 0 }));
    const log = document.getElementById("log");
    const count = document.getElementById("count");
    let total = 0, lap = -1;
    const PLAYER_Y = 0.6, SPAN = 5, SPEED = 2.1;

    function print(text) {
      if (reduced) return;
      log.querySelector(".empty")?.remove();
      const li = document.createElement("li");
      li.innerHTML = `<span>›</span>${text}<em class="src">coin.py:24</em>`;
      log.appendChild(li);
      while (log.children.length > 4) log.firstElementChild.remove();
      total += 1;
      count.textContent = `${total} collected`;
    }

    const dragging = orbit(view, canvas, () => loop.redraw());
    const loop = animate(canvas, (t) => {
      const tt = reduced ? 2.4 : t;
      const legLen = (2 * SPAN) / SPEED;
      const n = Math.floor(tt / legLen);
      const k = (tt % legLen) / legLen;
      const px = n % 2 === 0 ? -SPAN + 2 * SPAN * k : SPAN - 2 * SPAN * k;
      if (n !== lap) { lap = n; for (const c of coins) c.collected = false; }
      if (!dragging() && !reduced) view.yaw = 0.42 + Math.sin(tt * 0.15) * 0.12;
      const player = [px, PLAYER_Y, 0];

      view.begin();
      view.grid(9, 9.5, 0.36);
      view.line([-SPAN, 0.01, 0], [SPAN, 0.01, 0], COL.text, 0.28, [2, 6]);
      for (const c of coins) {
        const [hx, hy, hz] = c.home;
        const dy = PLAYER_Y - hy;
        const flat = Math.sqrt(Math.max(0, P.radius * P.radius - dy * dy));
        const inside = Math.hypot(px - hx, dy, -hz) < P.radius;
        view.ring([hx, 0, hz], flat, c.collected || inside ? COL.coin : COL.dim, c.collected ? 0.35 : inside ? 0.9 : 0.55);
        // the script: collect when the player is within PICKUP_RADIUS
        if (!c.collected && !reduced && Math.hypot(px - hx, PLAYER_Y - hy, 0 - hz) < P.radius) {
          c.collected = true; c.at = tt; print("+1");
        }
      }
      view.mesh(BOX, { pos: [px, PLAYER_Y, 0], scale: [0.6, 1.2, 0.6], yaw: n % 2 ? Math.PI : 0, color: COL.text, glow: 1 });
      for (const c of coins) {
        const [hx, hy, hz] = c.home;
        const age = c.collected ? tt - c.at : -1;
        if (age > 0.4) continue;
        const s = age >= 0 ? 1 - age / 0.4 : 1;
        const bob = P.amp * Math.sin(tt * P.freq + hx);
        view.mesh(COIN, { pos: [hx, hy + bob + Math.max(0, age) * 2, hz], scale: [0.7 * s, 0.7 * s, 0.13 * s], yaw: tt * P.spin, color: COL.coin, glow: 0.6 });
      }
      view.end([6, 20]);
      view.gizmo([player[0], player[1], player[2]], 0.9);
      coins.forEach((c, i) => view.label([c.home[0], 0, c.home[2]], `Coin ${i + 1}`, COL.faint, 0.9, 8, 16, 11));
    });

    // Draggable numbers: horizontal drag, or arrow keys when focused.
    const nums = [...document.querySelectorAll(".num")];
    const show = (el) => {
      const k = el.dataset.k;
      el.textContent = P[k].toFixed(+el.dataset.dec);
      el.setAttribute("aria-valuenow", P[k].toFixed(+el.dataset.dec));
    };
    for (const el of nums) {
      const k = el.dataset.k, min = +el.dataset.min, max = +el.dataset.max, step = +el.dataset.step;
      el.setAttribute("role", "slider");
      el.setAttribute("aria-valuemin", min);
      el.setAttribute("aria-valuemax", max);
      show(el);
      const set = (v) => { P[k] = clamp(Math.round(v / step) * step, min, max); show(el); if (reduced) loop.redraw(); };
      let drag = null;
      el.addEventListener("pointerdown", (e) => {
        drag = { x: e.clientX, v: P[k] };
        el.setPointerCapture(e.pointerId);
        el.classList.add("active");
        e.preventDefault();
      });
      el.addEventListener("pointermove", (e) => { if (drag) set(drag.v + ((e.clientX - drag.x) / 220) * (max - min)); });
      const end = () => { drag = null; el.classList.remove("active"); };
      el.addEventListener("pointerup", end);
      el.addEventListener("pointercancel", end);
      el.addEventListener("keydown", (e) => {
        const big = e.shiftKey ? 10 : 1;
        if (e.key === "ArrowRight" || e.key === "ArrowUp") { set(P[k] + step * big); e.preventDefault(); }
        else if (e.key === "ArrowLeft" || e.key === "ArrowDown") { set(P[k] - step * big); e.preventDefault(); }
        else if (e.key === "Home") { set(min); e.preventDefault(); }
        else if (e.key === "End") { set(max); e.preventDefault(); }
      });
    }
    document.getElementById("reset")?.addEventListener("click", () => {
      Object.assign(P, DEFAULTS);
      nums.forEach(show);
      loop.redraw();
    });
  })();

  // ---------- Oberon tour ----------
  (function tour() {
    const stops = [...document.querySelectorAll("#stops button")];
    const imgs = [...document.querySelectorAll("#shot img")];
    const focus = document.getElementById("focus");
    const caption = document.getElementById("caption");
    if (!stops.length || !focus) return;
    const select = (btn) => {
      for (const b of stops) b.setAttribute("aria-pressed", String(b === btn));
      for (const img of imgs) img.classList.toggle("on", img.dataset.name === btn.dataset.img);
      const [x, y, w, h] = btn.dataset.rect.split(" ").map(Number);
      Object.assign(focus.style, { left: `${(x / 1600) * 100}%`, top: `${(y / 950) * 100}%`, width: `${(w / 1600) * 100}%`, height: `${(h / 950) * 100}%` });
      focus.classList.toggle("below", y < 80);
      focus.firstElementChild.textContent = btn.querySelector(".name").textContent;
      caption.textContent = btn.querySelector(".desc").textContent;
    };
    for (const b of stops) {
      b.addEventListener("click", () => select(b));
      b.addEventListener("pointerenter", (e) => { if (e.pointerType === "mouse") select(b); });
      b.addEventListener("focus", () => select(b));
    }
    select(stops[0]);
  })();

  // ---------- header, reveals, copy, footer wordmark ----------
  const top = document.getElementById("top");
  const onScroll = () => top.classList.toggle("scrolled", scrollY > 24);
  addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
  }, { rootMargin: "0px 0px -8% 0px" });
  document.querySelectorAll(".rise").forEach((el) => io.observe(el));

  const copy = document.getElementById("copy");
  copy?.addEventListener("click", async () => {
    const text = [...document.getElementById("cmds").childNodes]
      .filter((n) => !(n.classList && n.classList.contains("p")))
      .map((n) => n.textContent).join("");
    try { await navigator.clipboard.writeText(text); copy.textContent = "Copied"; }
    catch { copy.textContent = "Select and copy"; }
    setTimeout(() => (copy.textContent = "Copy"), 1600);
  });

  // Fit the footer wordmark to the column width without distorting the letters.
  const giant = document.getElementById("giant");
  const fit = () => {
    giant.style.fontSize = "100px";
    const cs = getComputedStyle(giant.parentElement);
    const w = giant.parentElement.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    giant.style.fontSize = `${(100 * w) / giant.scrollWidth}px`;
  };
  document.fonts.ready.then(fit);
  new ResizeObserver(() => fit()).observe(giant.parentElement);
})();
