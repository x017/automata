// AUTOMATA dither art: one-bit, ordered (Bayer 8x8) dithering in the brand's
// black and off-white. Shared by the landing page and the docs.
//
//   Dither.scene(canvas, opts)  live ray-marched scene (WebGL), the hero art
//   Dither.develop(img)         screenshot starts dithered, dissolves to colour
//   Dither.swatch(canvas, a, b) two-tone dithered sky/ground swatch
//   Dither.moon(canvas)         dithered lit sphere
//   Dither.wordmark(canvas, t)  text filled with a dithered fade
//
// Everything is decoration: pages work without it, and it honours
// prefers-reduced-motion (art renders once, nothing animates).
(() => {
  "use strict";

  const INK = [242, 240, 235];
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // The same 8x8 Bayer threshold the shader uses, in 0..1.
  function bayer2(x, y) { return ((x * 0.5 + y * y * 0.75) % 1 + 1) % 1; }
  function bayer(x, y) {
    x = Math.floor(x); y = Math.floor(y);
    const b2 = (a, c) => bayer2(a & 1, c & 1);
    return b2(x >> 2, y >> 2) * 0.0625 + b2(x >> 1, y >> 1) * 0.25 + b2(x, y);
  }

  // ------------------------------------------------------------------ scene
  const VERT = "attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}";
  const FRAG = `
precision highp float;
uniform vec2 uRes;
uniform float uTime;
uniform vec2 uSun;     // pointer, -1..1: x swings the sun, y raises it
uniform float uOrbit;  // camera angle offset
uniform float uInk;    // 1 = ink on black, 0 = black ink on paper

float bayer2(vec2 a){ a = floor(a); return fract(dot(a, vec2(.5, a.y * .75))); }
float bayer8(vec2 a){ return bayer2(.25 * a) * .0625 + bayer2(.5 * a) * .25 + bayer2(a); }

mat2 rot(float a){ float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }
float sdBox(vec3 p, vec3 b, float r){ vec3 q = abs(p) - (b - r); return length(max(q, 0.)) + min(max(q.x, max(q.y, q.z)), 0.) - r; }
float sdCapsule(vec3 p, vec3 a, vec3 b, float r){ vec3 pa = p - a, ba = b - a; float h = clamp(dot(pa, ba) / dot(ba, ba), 0., 1.); return length(pa - ba * h) - r; }
float sdCoin(vec3 p){ vec2 d = vec2(length(p.xy) - .3, abs(p.z) - .035); return min(max(d.x, d.y), 0.) + length(max(d, 0.)) - .02; }

vec2 pick(vec2 a, float d, float m){ return d < a.x ? vec2(d, m) : a; }

vec2 coin(vec2 r, vec3 p, vec3 c, float i){
  vec3 q = p - c - vec3(0., .12 * sin(uTime * 1.6 + i * 1.7), 0.);
  q.xz = rot(uTime * 1.8 + i) * q.xz;
  return pick(r, sdCoin(q), 2.);
}

// x: distance, y: material (0 ground, 1 solid, 2 coin)
vec2 map(vec3 p){
  vec2 r = vec2(p.y, 0.);
  r = pick(r, sdBox(p - vec3(-2.3, .65, -.9), vec3(.65), .06), 1.);
  r = pick(r, sdBox(p - vec3(2.7, 1.1, -2.6), vec3(.45, 1.1, .45), .06), 1.);
  vec3 q = p - vec3(1.3, .38, 1.3); q.xz = rot(.55) * q.xz;
  r = pick(r, sdBox(q, vec3(.38), .05), 1.);
  r = pick(r, sdBox(p - vec3(-.6, .28, -3.4), vec3(1.7, .28, .95), .05), 1.);
  r = pick(r, sdBox(p - vec3(-5.4, .9, -3.2), vec3(.5, .9, .5), .06), 1.);
  r = pick(r, length(p - vec3(-3.9, .55, 1.7)) - .55, 1.);
  r = pick(r, sdCapsule(p, vec3(.2, .45, -.2), vec3(.2, 1.15, -.2), .42), 1.);
  r = coin(r, p, vec3(-.6, 1.75, -3.4), 0.);
  r = coin(r, p, vec3(1.3, 1.25, 1.3), 1.);
  r = coin(r, p, vec3(-2.3, 1.95, -.9), 2.);
  r = coin(r, p, vec3(3.6, .9, .6), 3.);
  r = coin(r, p, vec3(-5.4, 2.4, -3.2), 4.);
  return r;
}

vec3 normal(vec3 p){
  vec2 e = vec2(1e-3, 0.);
  return normalize(vec3(map(p + e.xyy).x - map(p - e.xyy).x,
                        map(p + e.yxy).x - map(p - e.yxy).x,
                        map(p + e.yyx).x - map(p - e.yyx).x));
}

float shadow(vec3 ro, vec3 rd){
  float res = 1., t = .03;
  for (int i = 0; i < 36; i++) {
    float h = map(ro + rd * t).x;
    res = min(res, clamp(10. * h / t, 0., 1.));
    t += clamp(h, .03, .6);
    if (res < .01 || t > 20.) break;
  }
  return res;
}

float occlusion(vec3 p, vec3 n){
  float o = 0., s = 1.;
  for (int i = 0; i < 5; i++) {
    float h = .02 + .12 * float(i);
    o += (h - map(p + n * h).x) * s;
    s *= .8;
  }
  return clamp(1. - 2.2 * o, 0., 1.);
}

void main(){
  vec2 frag = gl_FragCoord.xy;
  vec2 uv = (frag - .5 * uRes) / uRes.y;

  float a = .75 + uOrbit + uSun.x * .2;
  vec3 target = vec3(0., 1.25, -.9);
  vec3 ro = target + vec3(sin(a) * 8.8, 1.9 - uSun.y * .5, cos(a) * 8.8);
  vec3 fw = normalize(target - ro);
  vec3 rt = normalize(cross(fw, vec3(0., 1., 0.)));
  vec3 up = cross(rt, fw);
  float focal = 1. / tan(radians(21.)) * .5;
  vec3 rd = normalize(fw * focal + rt * uv.x + up * uv.y);

  float az = a - 1.05 + uSun.x * .6;
  float el = .5 + uSun.y * .18;
  vec3 sun = vec3(cos(el) * sin(az), sin(el), cos(el) * cos(az));

  float t = 0.;
  vec2 h = vec2(1., 0.);
  for (int i = 0; i < 110; i++) {
    h = map(ro + rd * t);
    if (abs(h.x) < 1e-3 || t > 45.) break;
    t += h.x * .9;
  }

  float horizon = 0.;
  float sky = .12 * pow(1. - clamp(rd.y / .25, 0., 1.), 6.);
  float lum = rd.y < 0. ? horizon : sky;
  if (t < 45.) {
    vec3 p = ro + rd * t;
    vec3 n = normal(p);
    float sh = shadow(p + n * .02, sun);
    float dif = clamp(dot(n, sun), 0., 1.) * sh;
    float checker = mod(floor(p.x) + floor(p.z), 2.);
    float albedo = h.y < .5 ? mix(.62, .46, checker) : (h.y < 1.5 ? .9 : 1.);
    float amb = (.03 + .04 * n.y) * occlusion(p, n);
    float col = albedo * (1.35 * dif + amb);
    if (h.y > 1.5) col += .9 * pow(clamp(dot(reflect(rd, n), sun), 0., 1.), 20.) * sh + .25;
    float pool = smoothstep(9.5, 2.5, length(p.xz - vec2(-.2, -1.)));
    lum = col * (h.y < .5 ? pool : mix(.55, 1., pool));
  }
  lum = clamp(lum, 0., 1.);
  lum = lum * lum * (3. - 2. * lum);
  float on = step(bayer8(vec2(frag.x, uRes.y - frag.y)), lum);
  on = mix(1. - on, on, uInk);
  gl_FragColor = vec4(mix(vec3(0.), vec3(${INK.map((c) => (c / 255).toFixed(4)).join(",")}), on), 1.);
}`;

  function compile(gl, type, src) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  }

  /**
   * Live scene in `canvas`. Options: pixel (CSS px per dither dot, 2),
   * time (fixed start time), orbit (camera offset), sun ([x, y] in -1..1),
   * still (render one frame), ink (false = paper). Returns a controller with
   * pause(), play(), and the `ok` flag (false when WebGL is unavailable).
   */
  function scene(canvas, opts = {}) {
    let pixel = opts.pixel || 2;
    const gl = canvas.getContext("webgl", { antialias: false, preserveDrawingBuffer: !!opts.still });
    const ctl = { ok: false, playing: false, pause() {}, play() {} };
    if (!gl) return ctl;
    let program;
    try {
      program = gl.createProgram();
      gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERT));
      gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAG));
      gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
    } catch (e) {
      console.warn("dither scene disabled:", e.message);
      return ctl;
    }
    gl.useProgram(program);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(program, "p");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const u = (n) => gl.getUniformLocation(program, n);
    const uRes = u("uRes"), uTime = u("uTime"), uSun = u("uSun"), uOrbit = u("uOrbit"), uInk = u("uInk");

    const sun = (opts.sun || [0, 0]).slice();
    const want = sun.slice();
    let time = opts.time ?? 3;
    let last = 0, raf = 0, visible = true, slow = 0;

    function size() {
      const r = canvas.getBoundingClientRect();
      const w = Math.max(1, Math.round(r.width / pixel));
      const h = Math.max(1, Math.round(r.height / pixel));
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    }
    function draw() {
      size();
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.uniform2f(uRes, canvas.width, canvas.height);
      gl.uniform1f(uTime, time);
      gl.uniform2f(uSun, sun[0], sun[1]);
      gl.uniform1f(uOrbit, (opts.orbit || 0) + time * 0.03);
      gl.uniform1f(uInk, opts.ink === false ? 0 : 1);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      canvas.classList.add("ready");
    }
    function frame(now) {
      raf = 0;
      const dt = Math.min(0.05, (now - (last || now)) / 1000);
      last = now;
      if (ctl.playing) time += dt;
      // Adaptive resolution: coarser dots if this machine can't keep up.
      if (dt > 0) {
        slow = slow * 0.9 + (dt > 0.034 ? 1 : 0) * 0.1;
        if (slow > 0.6 && pixel < 5) { pixel += 1; slow = 0; }
      }
      sun[0] += (want[0] - sun[0]) * Math.min(1, dt * 4);
      sun[1] += (want[1] - sun[1]) * Math.min(1, dt * 4);
      draw();
      if ((ctl.playing || Math.abs(want[0] - sun[0]) + Math.abs(want[1] - sun[1]) > 1e-3) && visible) {
        raf = requestAnimationFrame(frame);
      }
    }
    const kick = () => { if (!raf && visible) { last = 0; raf = requestAnimationFrame(frame); } };
    ctl.ok = true;
    ctl.play = () => { ctl.playing = true; kick(); };
    ctl.pause = () => { ctl.playing = false; };
    ctl.sun = (x, y) => { want[0] = x; want[1] = y; if (reduced) { sun[0] = x; sun[1] = y; } kick(); };
    ctl.draw = draw;

    draw();
    if (opts.still) return ctl;
    new ResizeObserver(() => { if (!raf) draw(); }).observe(canvas);
    new IntersectionObserver(([e]) => { visible = e.isIntersecting; if (visible) kick(); }).observe(canvas);
    document.addEventListener("visibilitychange", () => { visible = !document.hidden; if (visible) kick(); });
    if (!reduced) ctl.play();
    return ctl;
  }

  // ---------------------------------------------------------------- develop
  /**
   * The image first appears as a one-bit dither of itself, then dissolves to
   * full colour in Bayer order, when it scrolls into view. Needs the image on
   * the same origin (canvas readback); otherwise it's left alone.
   */
  function develop(img, opts = {}) {
    if (reduced || !("IntersectionObserver" in window)) return;
    const pixel = opts.pixel || 2;
    const wrap = document.createElement("span");
    wrap.className = "develop";
    img.replaceWith(wrap);
    wrap.appendChild(img);
    const canvas = document.createElement("canvas");
    canvas.setAttribute("aria-hidden", "true");
    wrap.appendChild(canvas);
    let data = null, ctx = null, thresholds = null;

    function prepare() {
      const w = Math.max(1, Math.round(img.getBoundingClientRect().width / pixel));
      const h = Math.max(1, Math.round(w * img.naturalHeight / img.naturalWidth));
      canvas.width = w; canvas.height = h;
      ctx = canvas.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(img, 0, 0, w, h);
      try { data = ctx.getImageData(0, 0, w, h); } catch { wrap.replaceWith(img); return false; }
      thresholds = new Float32Array(w * h);
      const px = data.data;
      for (let y = 0, i = 0; y < h; y++) {
        for (let x = 0; x < w; x++, i++) {
          const o = i * 4;
          // Lift the dark UI so the dither has something to say, then threshold.
          const l = Math.min(1, Math.pow((0.2126 * px[o] + 0.7152 * px[o + 1] + 0.0722 * px[o + 2]) / 255, 0.7) * 1.25);
          const b = bayer(x, y);
          const on = l > b;
          px[o] = on ? INK[0] : 0; px[o + 1] = on ? INK[1] : 0; px[o + 2] = on ? INK[2] : 0; px[o + 3] = 255;
          thresholds[i] = b;
        }
      }
      ctx.putImageData(data, 0, 0);
      return true;
    }
    function dissolve() {
      const start = performance.now() + (opts.hold ?? 350);
      const dur = opts.duration ?? 1100;
      const px = data.data;
      function step(now) {
        const k = Math.max(0, Math.min(1, (now - start) / dur));
        const e = k * k * (3 - 2 * k);
        for (let i = 0; i < thresholds.length; i++) px[i * 4 + 3] = thresholds[i] < e ? 0 : 255;
        ctx.putImageData(data, 0, 0);
        if (k < 1) requestAnimationFrame(step); else canvas.remove();
      }
      requestAnimationFrame(step);
    }
    const ready = () => img.complete && img.naturalWidth > 0;
    const io = new IntersectionObserver(([e]) => {
      if (!e.isIntersecting || !ready()) return;
      io.disconnect();
      dissolve();
    }, { rootMargin: "0px 0px -15% 0px" });
    const go = () => { if (prepare()) io.observe(wrap); };
    if (ready()) go(); else img.addEventListener("load", go, { once: true });
  }

  // ------------------------------------------------------------------- 2D art
  function paint(canvas, fn) {
    const w = canvas.width, h = canvas.height;
    const ctx = canvas.getContext("2d");
    const img = ctx.createImageData(w, h);
    for (let y = 0, i = 0; y < h; y++) {
      for (let x = 0; x < w; x++, i += 4) {
        const c = fn(x / w, y / h, bayer(x, y), x, y);
        if (!c) continue;
        img.data[i] = c[0]; img.data[i + 1] = c[1]; img.data[i + 2] = c[2]; img.data[i + 3] = 255;
      }
    }
    ctx.putImageData(img, 0, 0);
  }
  const hex = (s) => [1, 3, 5].map((i) => parseInt(s.slice(i, i + 2), 16));

  /** A sky over a horizon with a low sun, dithered between two colours. */
  function swatch(canvas, dark, light) {
    const a = hex(dark), b = hex(light);
    paint(canvas, (u, v, t) => {
      const horizon = 0.64;
      let l;
      if (v < horizon) {
        l = 0.06 + 0.8 * Math.pow(v / horizon, 2.4);
        const r = Math.hypot((u - 0.68) * 1.66, v - horizon);
        if (r < 0.16) l = 1;
      } else {
        l = 0.42 * (1 - (v - horizon) / (1 - horizon)) ** 1.5;
      }
      return l > t ? b : a;
    });
  }

  /** A sphere lit from the upper left, with a dithered terminator. */
  function moon(canvas, light = [-0.55, 0.45, 0.7]) {
    const n = Math.hypot(...light);
    const L = light.map((c) => c / n);
    paint(canvas, (u, v, t) => {
      const x = u * 2 - 1, y = 1 - v * 2;
      const d = x * x + y * y;
      if (d > 1) return null;
      const z = Math.sqrt(1 - d);
      const crater = 0.08 * Math.sin(x * 9.0 + Math.sin(y * 7)) * Math.cos(y * 8 - x * 3);
      const l = Math.max(0, x * L[0] + y * L[1] + z * L[2]) * (0.92 + crater) + 0.02;
      return Math.pow(l, 1.25) > t ? INK : [0, 0, 0];
    });
  }

  /** `text` set in Geist at the canvas width, filled with a dithered fade. Sets the height. */
  function wordmark(canvas, text) {
    const w = canvas.width;
    const ctx = canvas.getContext("2d");
    ctx.font = "700 100px Geist, sans-serif";
    const size = 100 * w / (ctx.measureText(text).width || 1) * 0.99;
    const h = Math.ceil(size * 0.74);
    canvas.height = h;
    ctx.font = `700 ${size}px Geist, sans-serif`;
    ctx.letterSpacing = "0px";
    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = "#fff";
    ctx.fillText(text, w * 0.002, h - 1);
    const img = ctx.getImageData(0, 0, w, h);
    const px = img.data;
    for (let y = 0, i = 0; y < h; y++) {
      const l = 0.92 - 0.88 * (y / h);
      for (let x = 0; x < w; x++, i += 4) {
        const on = px[i + 3] > 127 && l > bayer(x, y);
        px[i] = INK[0]; px[i + 1] = INK[1]; px[i + 2] = INK[2]; px[i + 3] = on ? 255 : 0;
      }
    }
    ctx.putImageData(img, 0, 0);
  }

  window.Dither = { scene, develop, swatch, moon, wordmark, bayer, reduced };
})();
