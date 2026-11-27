// Plays Coin Run in headless Chromium the way the Android emulator test
// does (android/ci/coin_run_test.sh): the web player loads the exported
// game from game.zip and runs it with automata.player.
//
// 1. it starts: game.zip unpacks, the level loads, its scripts run and
//    say the keyboard plays (no touch controls on a computer);
// 2. keys reach the scripts: F2 cycles the graphics preset;
// 3. the setting lands in IndexedDB (/data/Coin Run/settings.json);
// 4. at the low preset (SwiftShader draws on the CPU; at the top presets
//    the game runs slow), once the intro has flown the camera behind the
//    player, walking forward (W) picks up the coin ahead, and the best
//    score is saved too;
// 5. after a reload the game remembers: F2 goes on from the saved preset
//    and it says "Welcome back!" with the best run;
// 6. with ?touch=1 (a phone) the touch controls show by themselves.
//
// It also checks every draw's uniform buffers are as big as the shader
// expects (soft). Writes report.txt, stats.json, console.txt and
// screenshots (1-start, 2-walked, 3-touch) to OUT
// (default web/dist/coin_run/).
//
//   node web/ci/coin_run.mjs URL
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const url = process.argv[2] ?? "http://127.0.0.1:8001/";
const out = process.env.OUT ?? "web/dist/coin_run";
fs.mkdirSync(out, { recursive: true });

const PRESETS = ["low", "medium", "high", "ultra", "beautiful"];
const SETTINGS = "/data/Coin Run/settings.json";

const browser = await chromium.launch({
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
// One context for every load: IndexedDB outlives a reload, as for players.
const context = await browser.newContext({ viewport: { width: 960, height: 540 } });
// WebGL2 refuses a draw whose uniform buffer is smaller than the shader's
// block ("uniform buffer that is too small"). Before each draw this
// compares every active block with the range bound to it and keeps the
// first few that don't fit (window.__uniformProblems), so the report names
// the block instead of only Chromium's warning.
await context.addInitScript(() => {
  const problems = new Map();
  window.__uniformProblems = problems;
  const proto = WebGL2RenderingContext.prototype;
  let checked = 0;
  const inspect = (gl, call) => {
    if (checked++ > 20000) return;
    const program = gl.getParameter(gl.CURRENT_PROGRAM);
    if (!program) return;
    const blocks = gl.getProgramParameter(program, gl.ACTIVE_UNIFORM_BLOCKS);
    for (let i = 0; i < blocks; i++) {
      const need = gl.getActiveUniformBlockParameter(program, i, gl.UNIFORM_BLOCK_DATA_SIZE);
      const binding = gl.getActiveUniformBlockParameter(program, i, gl.UNIFORM_BLOCK_BINDING);
      const have = gl.getIndexedParameter(gl.UNIFORM_BUFFER_SIZE, binding);
      if (have !== 0 && have < need) {
        const name = gl.getActiveUniformBlockName(program, i);
        const key = `${call} ${name} need ${need} have ${have}`;
        if (!problems.has(key)) {
          const indices = gl.getActiveUniformBlockParameter(program, i, gl.UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES);
          const members = Array.from(indices, (u) => gl.getActiveUniform(program, u).name).slice(0, 6);
          problems.set(key, { call, name, need, have, binding, members, count: 0 });
        }
        problems.get(key).count += 1;
      }
    }
  };
  for (const call of ["drawArraysInstanced", "drawElementsInstanced", "drawArrays", "drawElements"]) {
    const original = proto[call];
    proto[call] = function (...args) {
      try {
        inspect(this, call);
      } catch {
        // a probe mustn't break the game
      }
      return original.apply(this, args);
    };
  }
});
const page = await context.newPage();
const lines = [];
page.on("console", (m) => lines.push(`[${m.type()}] ${m.text()}`));
page.on("pageerror", (e) => lines.push(`[pageerror] ${e.message}`));

const checks = [];
const check = (name, ok, detail, soft = false) => {
  checks.push({ name, ok: Boolean(ok), detail, soft });
  console.log(`${ok ? "ok  " : soft ? "warn" : "FAIL"}  ${name}: ${detail}`);
  return Boolean(ok);
};
const stats = () => page.evaluate(() => window.automata ?? null);

// Waits for a console line (logged after `from`) matching `pattern`.
async function waitLine(pattern, from, timeout) {
  const end = Date.now() + timeout;
  for (;;) {
    const hit = lines.slice(from).find((l) => pattern.test(l));
    if (hit) return hit;
    const s = await stats();
    if (s?.phase === "error" || Date.now() > end) return null;
    await page.waitForTimeout(200);
  }
}

// Loads the page; returns the start line ("Collect N coins! …") or null.
async function load(target) {
  const from = lines.length;
  await page.goto(target);
  const line = await waitLine(/Collect \d+ coins/, from, 240_000);
  if (line) {
    await page.waitForFunction(() => window.automata.frames >= 20, null, { timeout: 60_000 }).catch(() => {});
  }
  return { line, from };
}

// Holds a key for `ms` (long enough for a slow frame to see it).
async function hold(key, ms) {
  await page.keyboard.down(key);
  await page.waitForTimeout(ms);
  await page.keyboard.up(key);
}

async function pressF2(from) {
  await hold("F2", 400);
  const line = await waitLine(/Graphics: [a-z]+/, from, 20_000);
  return line ? line.match(/Graphics: ([a-z]+)/)[1] : null;
}

const started = Date.now();
let s;

// 1. First start.
const first = await load(url);
const firstStarted = Date.now();
s = await stats();
if (check("starts", first.line, first.line ?? `no start line; phase ${s?.phase}, error ${s?.error}`)) {
  check("unpacks game.zip", s.timings?.unpacked !== undefined, `unpacked at ${s.timings?.unpacked} ms, title "${await page.title()}"`);
  check("keyboard controls on a computer", /WASD/.test(first.line) && !s.touchScreen, first.line);
  check("storage mounted", s.storage?.mounted, JSON.stringify(s.storage));
  await page.waitForTimeout(1500);
  await page.screenshot({ path: path.join(out, "1-start.png") });
  await page.mouse.click(480, 270); // focus, as a player's first click

  // 2. A key reaches the scripts.
  let from = lines.length;
  const chosen = await pressF2(from);
  check("F2 changes the graphics preset", chosen, `chose ${chosen}`);

  // 3. ctx.settings is written in the frame it changes, then synced.
  const synced = await page
    .waitForFunction((file) => window.automata.storage.files.includes(file), SETTINGS, { timeout: 15_000 })
    .then(() => true)
    .catch(() => false);
  s = await stats();
  check("settings kept in IndexedDB", synced, `files ${JSON.stringify(s.storage.files)}, syncs ${s.storage.syncs}`);

  // 4. Walk into the coin ahead, at "low" (as the emulator test does).
  let preset = chosen;
  for (let i = 0; i < PRESETS.length && preset && preset !== "low"; i++) {
    preset = await pressF2(lines.length);
  }
  check("F2 reaches the low preset", preset === "low", `at ${preset}`);
  // The intro flies the camera for about 6 s; then it's behind the player,
  // looking at coin 5 straight ahead (7 m away).
  await page.waitForTimeout(Math.max(0, 9000 - (Date.now() - firstStarted)));
  from = lines.length;
  await page.keyboard.down("w");
  const scored = await waitLine(/Score: [1-9]/, from, 45_000);
  await page.keyboard.up("w");
  s = await stats();
  check("walking forward picks up a coin", scored, scored ?? `no Score line in 45 s (${s.frames} frames)`);
  await page.waitForTimeout(1500); // let the save sync
  await page.screenshot({ path: path.join(out, "2-walked.png") });

  // 5. Reload: the setting (and the best score) come back.
  const again = await load(url);
  s = await stats();
  if (check("starts again after a reload", again.line, again.line ?? `phase ${s?.phase}`)) {
    check("finds the save at start", s.storage.files.includes(SETTINGS), JSON.stringify(s.storage.files));
    if (scored) {
      const back = await waitLine(/Welcome back! Best run: [1-9]/, again.from, 5_000);
      check("remembers the best run", back, back ?? "no Welcome back line");
    }
    await page.mouse.click(480, 270);
    const next = await pressF2(lines.length);
    const want = preset ? PRESETS[(PRESETS.indexOf(preset) + 1) % PRESETS.length] : null;
    check("remembers the graphics preset", next && next === want, `F2 chose ${next}, want ${want} (after ${preset})`);
  }

  // 6. A phone: the touch controls show by themselves.
  const touch = await load(`${url}?touch=1`);
  s = await stats();
  check("touch controls on a touch screen", touch.line && /Left stick/.test(touch.line) && s.touchScreen, touch.line ?? "no start line");
  await page.waitForTimeout(1500);
  await page.screenshot({ path: path.join(out, "3-touch.png") });
  await page.setViewportSize({ width: 640, height: 360 });
  await page.waitForTimeout(500);
  await page.screenshot({ path: path.join(out, "3-touch.jpg"), type: "jpeg", quality: 55 });
}
s = await stats();
const uniformProblems = await page.evaluate(() => [...(window.__uniformProblems?.values() ?? [])]).catch(() => []);
check(
  "uniform buffers fit their blocks",
  uniformProblems.length === 0,
  uniformProblems.length ? uniformProblems.map((p) => JSON.stringify(p)).join("; ") : "every draw",
  true,
);
await browser.close();

check("no errors", !s?.error && !lines.some((l) => l.startsWith("[pageerror]")), s?.error ?? "none");
const ok = checks.every((c) => c.ok || c.soft);
const report = [
  `result: ${ok ? "ok" : "FAILED"}`,
  `checks: ${checks.filter((c) => c.ok).length}/${checks.length} passed`,
  ...checks.map((c) => `  ${c.ok ? "ok  " : c.soft ? "warn" : "FAIL"}  ${c.name}: ${c.detail}`),
  `timings of the last load (ms): ${JSON.stringify(s?.timings ?? {})}`,
  `storage: ${JSON.stringify(s?.storage ?? {})}`,
  `wall clock: ${Date.now() - started} ms`,
  "",
  "console:",
  ...lines.slice(-150),
].join("\n");
fs.writeFileSync(path.join(out, "console.txt"), lines.join("\n"));
fs.writeFileSync(path.join(out, "stats.json"), JSON.stringify({ ok, checks, stats: s }, null, 2));
fs.writeFileSync(path.join(out, "report.txt"), report);
console.log(report);
process.exit(ok ? 0 : 1);
