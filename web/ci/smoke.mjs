// Loads the web player in headless Chromium (WebGL2 on SwiftShader) and
// checks that the game runs: frames advance, no errors. Writes what it saw
// to OUT (default web/dist/smoke/): console.txt, stats.json, report.txt,
// screenshot.png and a small screenshot.jpg for the check run.
//
//   node web/ci/smoke.mjs URL [SITE_DIR]
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";

const url = process.argv[2] ?? "http://127.0.0.1:8000/";
const site = process.argv[3];
const out = process.env.OUT ?? "web/dist/smoke";
const wantFrames = Number(process.env.WANT_FRAMES ?? 180);
fs.mkdirSync(out, { recursive: true });

const browser = await chromium.launch({
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
const page = await browser.newPage({ viewport: { width: 960, height: 540 } });
const consoleLines = [];
page.on("console", (m) => consoleLines.push(`[${m.type()}] ${m.text()}`));
page.on("pageerror", (e) => consoleLines.push(`[pageerror] ${e.message}`));

const started = Date.now();
await page.goto(url);
let stats;
try {
  await page.waitForFunction(
    (want) => window.automata && (window.automata.frames >= want || window.automata.phase === "error" || window.automata.phase === "finished"),
    wantFrames,
    { timeout: 240_000, polling: 250 },
  );
} catch (e) {
  consoleLines.push(`[smoke] timed out: ${e.message}`);
}
stats = await page.evaluate(() => window.automata ?? null);
// Frame rate over two seconds of steady running.
let fps = null;
if (stats && stats.phase === "running") {
  const before = await page.evaluate(() => window.automata.frames);
  await page.waitForTimeout(2000);
  const after = await page.evaluate(() => window.automata.frames);
  fps = (after - before) / 2;
  stats = await page.evaluate(() => window.automata);
}
const webgl = await page.evaluate(() => {
  const gl = document.createElement("canvas").getContext("webgl2");
  if (!gl) return "no WebGL2";
  const info = gl.getExtension("WEBGL_debug_renderer_info");
  return info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
});
await page.screenshot({ path: path.join(out, "screenshot.png") });
await page.setViewportSize({ width: 640, height: 360 });
await page.waitForTimeout(500);
await page.screenshot({ path: path.join(out, "screenshot.jpg"), type: "jpeg", quality: 55 });
await browser.close();

// Download size: every file, raw and gzipped (what a server sends).
const sizes = [];
if (site) {
  for (const name of fs.readdirSync(site, { recursive: true })) {
    const file = path.join(site, name);
    if (!fs.statSync(file).isFile()) continue;
    const data = fs.readFileSync(file);
    sizes.push({ name, bytes: data.length, gzip: zlib.gzipSync(data, { level: 9 }).length });
  }
  sizes.sort((a, b) => b.bytes - a.bytes);
}
const mb = (n) => (n / 1e6).toFixed(2);
const total = sizes.reduce((s, f) => s + f.bytes, 0);
const totalGz = sizes.reduce((s, f) => s + f.gzip, 0);

const ok = Boolean(stats && stats.frames >= Math.min(wantFrames, 60) && !stats.error);
const report = [
  `result: ${ok ? "ok" : "FAILED"}`,
  `phase: ${stats?.phase}   frames: ${stats?.frames}   fps (SwiftShader): ${fps}`,
  `error: ${stats?.error ?? "none"}`,
  `WebGL renderer: ${webgl}`,
  `timings (ms since navigation): ${JSON.stringify(stats?.timings ?? {})}`,
  `wall clock to the end of the check: ${Date.now() - started} ms`,
  "",
  `download: ${mb(total)} MB (${mb(totalGz)} MB gzipped), ${sizes.length} files`,
  ...sizes.slice(0, 12).map((f) => `  ${f.name.padEnd(48)} ${mb(f.bytes).padStart(7)} MB  gz ${mb(f.gzip).padStart(7)} MB`),
  "",
  "console:",
  ...consoleLines.slice(-150),
].join("\n");
fs.writeFileSync(path.join(out, "console.txt"), consoleLines.join("\n"));
fs.writeFileSync(path.join(out, "stats.json"), JSON.stringify({ ok, stats, fps, webgl, sizes, total, totalGz }, null, 2));
fs.writeFileSync(path.join(out, "report.txt"), report);
console.log(report);
process.exit(ok ? 0 : 1);
