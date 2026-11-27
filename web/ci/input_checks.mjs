// Drives the web test game (web/spike/main.py) with fake input in
// Playwright and checks the game saw it: keys and modifiers, letting go on
// blur, mouse position, clicks and the wheel, touches, a fake gamepad,
// pointer lock, a hidden tab, resizing, and sound reaching the page.
//
// The game writes what it saw to `window.automata.game` (JSON) each frame;
// the player keeps input and audio counters in `window.automata`.
// Returns a list of { name, ok, detail, soft } (soft: reported, not
// failed: what headless Chromium may not grant, like pointer lock).

// Installed before the page loads: a standard-mapping pad the checks plug
// in and press through `window.__testPad`.
export function fakeGamepadScript() {
  const pad = {
    id: "AUTOMATA test pad (STANDARD GAMEPAD)",
    index: 0,
    connected: false,
    mapping: "standard",
    timestamp: 0,
    buttons: Array.from({ length: 17 }, () => ({ pressed: false, touched: false, value: 0 })),
    axes: [0, 0, 0, 0],
  };
  window.__testPad = pad;
  Object.defineProperty(navigator, "getGamepads", {
    configurable: true,
    value: () => [pad.connected ? pad : null, null, null, null],
  });
}

export async function runInputChecks(page) {
  const results = [];
  const read = () =>
    page.evaluate(() => ({
      stats: JSON.parse(JSON.stringify(window.automata)),
      game: window.automata.game ? JSON.parse(window.automata.game) : null,
    }));
  // Waits until `test(stats, game)` holds (the game runs at ~10 fps on
  // SwiftShader, so generously).
  const until = async (test, timeout = 20_000) => {
    const end = Date.now() + timeout;
    let last;
    while (Date.now() < end) {
      last = await read();
      if (last.game && test(last.stats, last.game)) return { ok: true, ...last };
      await page.waitForTimeout(100);
    }
    return { ok: false, ...last };
  };
  const check = async (name, fn, soft = false) => {
    try {
      const { ok, detail } = await fn();
      results.push({ name, ok, detail, soft });
    } catch (error) {
      results.push({ name, ok: false, detail: `threw: ${error.message}`, soft });
    }
  };
  const near = (point, x, y, tolerance = 3) =>
    Array.isArray(point) && Math.abs(point[0] - x) <= tolerance && Math.abs(point[1] - y) <= tolerance;

  await page.locator("canvas").focus();

  await check("keys move the cube (KeyD held)", async () => {
    const before = (await read()).game;
    await page.keyboard.down("KeyD");
    const r = await until((s, g) => g.x > before.x + 0.3 && g.keys.includes("d"));
    await page.keyboard.up("KeyD");
    return { ok: r.ok, detail: `x ${before.x.toFixed(2)} -> ${r.game?.x.toFixed(2)}, keys ${r.game?.keys}` };
  });

  await check("modifiers (Shift held)", async () => {
    await page.keyboard.down("Shift");
    const down = await until((s, g) => g.shift === true);
    await page.keyboard.up("Shift");
    const up = await until((s, g) => g.shift === false);
    return { ok: down.ok && up.ok, detail: `held seen ${down.ok}, released seen ${up.ok}` };
  });

  await check("blur lets go of held keys", async () => {
    await page.keyboard.down("KeyA");
    const moving = await until((s, g) => g.keys.includes("a"));
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await page.waitForTimeout(600);
    const a = (await read()).game.x;
    await page.waitForTimeout(1200);
    const after = await read();
    await page.keyboard.up("KeyA");
    const still = Math.abs(after.game.x - a) < 0.05;
    return {
      ok: moving.ok && still && after.stats.input.focusLost >= 1,
      detail: `x after blur ${a.toFixed(2)} -> ${after.game.x.toFixed(2)}, focusLost ${after.stats.input.focusLost}`,
    };
  });

  await check("cursor position in canvas pixels", async () => {
    await page.mouse.move(480, 270);
    await page.mouse.move(481, 271);
    const r = await until((s, g) => near(g.cursor, 481, 271));
    return { ok: r.ok, detail: `cursor ${JSON.stringify(r.game?.cursor)}, motion ${r.game?.mouse_motion}` };
  });

  await check("click", async () => {
    const before = (await read()).game.clicks;
    await page.mouse.click(300, 200);
    const r = await until((s, g) => g.clicks > before && near(g.tap_at, 300, 200));
    return { ok: r.ok, detail: `clicks ${before} -> ${r.game?.clicks} at ${JSON.stringify(r.game?.tap_at)}` };
  });

  await check("wheel (away from you is +y)", async () => {
    await page.mouse.move(480, 270);
    await page.mouse.wheel(0, -300);
    const r = await until((s, g) => g.scroll > 0.5);
    return { ok: r.ok, detail: `scroll ${r.game?.scroll}, zoom ${r.game?.zoom}` };
  });

  await check("touch", async () => {
    const before = (await read()).game.touches_started;
    await page.touchscreen.tap(700, 400);
    const r = await until((s, g) => g.touches_started > before && near(g.touch_at, 700, 400));
    return {
      ok: r.ok && r.stats.input.touches >= 1,
      detail: `touches ${before} -> ${r.game?.touches_started} at ${JSON.stringify(r.game?.touch_at)}, page saw ${r.stats.input.touches}`,
    };
  });

  await check("gamepad: connect, stick, button, disconnect", async () => {
    await page.evaluate(() => (window.__testPad.connected = true));
    const connected = await until((s, g) => g.pad !== null && g.pad_events.includes("connected"));
    const x0 = connected.game?.x ?? 0;
    await page.evaluate(() => (window.__testPad.axes[0] = -1));
    const stick = await until((s, g) => g.pad && g.pad.stick[0] < -0.9 && g.x < x0 - 0.3);
    await page.evaluate(() => (window.__testPad.axes[0] = 0));
    const beeps = stick.game?.beeps ?? 0;
    await page.evaluate(() => {
      window.__testPad.buttons[0].pressed = true;
      window.__testPad.buttons[0].value = 1;
    });
    const button = await until((s, g) => g.pad && g.pad.south && g.beeps > beeps);
    await page.evaluate(() => {
      window.__testPad.buttons[0].pressed = false;
      window.__testPad.buttons[0].value = 0;
    });
    await page.evaluate(() => (window.__testPad.connected = false));
    const gone = await until((s, g) => g.pad === null && g.pad_events.includes("disconnected"));
    return {
      ok: connected.ok && stick.ok && button.ok && gone.ok,
      detail: `connected ${connected.ok} (${connected.game?.pad?.name}), stick ${stick.ok}, south ${button.ok}, disconnected ${gone.ok}; events ${gone.game?.pad_events}`,
    };
  });

  await check("sound: Space beeps and the mix reaches the page", async () => {
    await page.evaluate(() => (window.automata.audio.peak = 0));
    const before = await read();
    await page.keyboard.press("Space");
    const r = await until((s, g) => g.beeps > before.game.beeps && s.audio.peak > 0.2);
    return {
      ok: r.ok,
      detail: `beeps ${r.game?.beeps}, peak ${r.stats.audio.peak.toFixed(2)}, context ${r.stats.audio.state} @ ${r.stats.audio.rate} Hz`,
    };
  });

  await check(
    "sound: the AudioWorklet plays it",
    async () => {
      const r = await until((s) => s.audio.played > 0 && s.audio.posted > 0, 5_000);
      const a = r.stats.audio;
      return {
        ok: r.ok,
        detail: `state ${a.state}, posted ${a.posted}, played ${a.played}, queued ${a.queued}, dropped ${a.dropped}, underruns ${a.underruns}`,
      };
    },
    true,
  );

  await check("pointer lock is asked for (L) and given back", async () => {
    await page.keyboard.press("KeyL");
    const wanted = await until((s, g) => g.cursor_locked && s.pointer.wanted);
    await page.waitForTimeout(500);
    const mid = await read();
    await page.mouse.click(480, 270);
    await page.keyboard.press("KeyL");
    const freed = await until((s, g) => !g.cursor_locked && !s.pointer.wanted);
    await page.waitForTimeout(300);
    const end = await read();
    const p = mid.stats.pointer;
    return {
      ok: wanted.ok && freed.ok && !end.stats.pointer.locked,
      detail: `wanted ${wanted.ok}, browser locked ${p.locked} (requests ${p.requests}, errors ${p.errors}), freed ${freed.ok}`,
    };
  });

  await check("hidden tab: lets go, and resumes", async () => {
    await page.keyboard.down("KeyW");
    await until((s, g) => g.keys.includes("w"));
    const before = (await read()).stats.input;
    const setHidden = (hidden) =>
      page.evaluate((h) => {
        Object.defineProperty(document, "hidden", { configurable: true, get: () => h });
        Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (h ? "hidden" : "visible") });
        document.dispatchEvent(new Event("visibilitychange"));
      }, hidden);
    await setHidden(true);
    await page.waitForTimeout(500);
    const z = (await read()).game.z;
    await page.waitForTimeout(1000);
    const hiddenZ = (await read()).game.z;
    await setHidden(false);
    await page.keyboard.up("KeyW");
    const r = await until((s) => s.input.resumed > before.resumed);
    const frames = r.game?.frame ?? 0;
    const later = await until((s, g) => g.frame > frames + 3);
    return {
      ok: r.ok && later.ok && Math.abs(hiddenZ - z) < 0.05 && r.stats.input.focusLost > before.focusLost,
      detail: `z while hidden ${z.toFixed(2)} -> ${hiddenZ.toFixed(2)}, focusLost ${r.stats.input.focusLost}, resumed ${r.stats.input.resumed}, frames go on ${later.ok}`,
    };
  });

  await check("resize follows the window", async () => {
    await page.setViewportSize({ width: 800, height: 600 });
    const r = await until((s, g) => g.size[0] === 800 && g.size[1] === 600);
    await page.setViewportSize({ width: 960, height: 540 });
    const back = await until((s, g) => g.size[0] === 960 && g.size[1] === 540);
    return { ok: r.ok && back.ok, detail: `size ${JSON.stringify(r.game?.size)} then ${JSON.stringify(back.game?.size)}` };
  });

  return results;
}
