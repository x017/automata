// The AUTOMATA web player: loads Pyodide (bundled beside this file), the
// engine's wheel and the game, then runs one frame per animation frame.
//
// `player.json` names the wheel and the game's entry script. The game's
// `game.run()` only starts it (on the web the browser owns the loop); this
// file then calls `automata._core.web_frame(width, height)` each frame
// until it returns false. Between frames it passes the page's input on
// (keys, mouse, pointer lock, touches, gamepads), tells the engine when
// the tab is hidden, and plays the sound the engine mixed through an
// AudioWorklet (audio-worklet.js).
//
// `window.automata` holds what CI reads: the phase, timings in
// milliseconds since navigation, frame count, errors, input and audio
// counters, and `game` (whatever the game writes there).

import { loadPyodide } from "./pyodide/pyodide.mjs";

const canvas = document.getElementById("canvas");
const status = document.getElementById("status");
const stats = (window.automata = {
  phase: "loading",
  timings: {},
  frames: 0,
  error: null,
  input: { keys: 0, unknownKeys: 0, mouse: 0, touches: 0, wheel: 0, focusLost: 0, resumed: 0, pads: 0 },
  pointer: { wanted: false, locked: false, requests: 0, errors: 0 },
  audio: { state: "off", rate: 0, posted: 0, peak: 0, played: 0, queued: 0, dropped: 0, underruns: 0 },
  game: null,
});

function mark(name) {
  stats.timings[name] = Math.round(performance.now());
}

function fail(error) {
  stats.phase = "error";
  stats.error = String(error && error.message ? error.message : error);
  status.textContent = stats.error;
  status.className = "error";
  console.error(error);
}

// The canvas's drawing buffer follows its CSS size at the screen's pixel
// ratio; the engine draws at that size.
function fitCanvas() {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
}

// A page position in canvas (drawing buffer) pixels, as the engine counts.
function canvasPoint(event) {
  const rect = canvas.getBoundingClientRect();
  const sx = rect.width > 0 ? canvas.width / rect.width : 1;
  const sy = rect.height > 0 ? canvas.height / rect.height : 1;
  return [(event.clientX - rect.left) * sx, (event.clientY - rect.top) * sy];
}

// --- Sound ------------------------------------------------------------

const sound = { context: null, node: null };

async function startAudio(rate) {
  stats.audio.rate = rate;
  if (!window.AudioContext || !window.AudioWorkletNode) {
    stats.audio.state = "unsupported";
    return;
  }
  try {
    const context = new AudioContext({ sampleRate: rate, latencyHint: "interactive" });
    await context.audioWorklet.addModule(new URL("audio-worklet.js", import.meta.url).href);
    const node = new AudioWorkletNode(context, "automata-output", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    node.connect(context.destination);
    node.port.onmessage = (event) => Object.assign(stats.audio, event.data);
    context.onstatechange = () => (stats.audio.state = context.state);
    sound.context = context;
    sound.node = node;
    stats.audio.state = context.state;
  } catch (error) {
    stats.audio.state = "failed";
    console.warn("automata: no sound:", error);
  }
}

// Browsers start sound only after the player touches the page.
function unlockAudio() {
  if (sound.context && sound.context.state === "suspended" && !document.hidden) {
    sound.context.resume().catch(() => {});
  }
}

// Takes what the engine mixed this frame; plays it while sound runs.
function pumpAudio(core) {
  const proxy = core.web_audio();
  let bytes;
  try {
    bytes = proxy.toJs();
  } finally {
    proxy.destroy();
  }
  if (bytes.byteLength < 8) return;
  const samples = new Float32Array(bytes.byteLength / 4);
  new Uint8Array(samples.buffer).set(bytes);
  let peak = stats.audio.peak;
  for (let i = 0; i < samples.length; i += 1) {
    const v = Math.abs(samples[i]);
    if (v > peak) peak = v;
  }
  stats.audio.peak = peak;
  if (sound.node && sound.context.state === "running") {
    stats.audio.posted += samples.length / 2;
    sound.node.port.postMessage(samples, [samples.buffer]);
  }
}

// --- Input ------------------------------------------------------------

function installInput(pyodide, core) {
  const call = (name) => {
    const fn = core[name];
    return (...args) => {
      try {
        return fn(...args);
      } catch (error) {
        console.warn(`automata: ${name}:`, error);
        return false;
      }
    };
  };
  const key = call("web_key");
  const modifiers = call("web_modifiers");
  const mouseButton = call("web_mouse_button");
  const cursor = call("web_cursor");
  const mouseMotion = call("web_mouse_motion");
  const scroll = call("web_scroll");
  const touch = call("web_touch");
  const focusLost = call("web_focus_lost");
  const resume = call("web_resume");
  const gamepads = core.web_gamepads ? call("web_gamepads") : null;

  let heldModifiers = 0;
  const heldButtons = new Set();
  const modifierBits = (e) => (e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.altKey ? 4 : 0) | (e.metaKey ? 8 : 0);
  const syncModifiers = (e) => {
    const bits = modifierBits(e);
    if (bits !== heldModifiers) {
      heldModifiers = bits;
      modifiers(bits);
    }
  };

  const onKey = (down) => (e) => {
    syncModifiers(e);
    if (down) unlockAudio();
    if (key(e.code, down)) {
      stats.input.keys += 1;
      // The game's keys mustn't scroll the page or move focus, but
      // browser shortcuts (Ctrl/Cmd+…, F5, F11, F12) stay the browser's.
      if (!e.ctrlKey && !e.metaKey && !/^F\d+$/.test(e.code)) e.preventDefault();
      if (down) requestLockIfWanted();
    } else {
      stats.input.unknownKeys += 1;
    }
  };
  window.addEventListener("keydown", onKey(true));
  window.addEventListener("keyup", onKey(false));

  canvas.addEventListener("mousedown", (e) => {
    canvas.focus();
    unlockAudio();
    requestLockIfWanted();
    syncModifiers(e);
    heldButtons.add(e.button);
    mouseButton(e.button, true);
    stats.input.mouse += 1;
    e.preventDefault();
  });
  // Released anywhere (even off the canvas) is released.
  window.addEventListener("mouseup", (e) => {
    if (heldButtons.delete(e.button)) mouseButton(e.button, false);
  });
  window.addEventListener("mousemove", (e) => {
    if (document.pointerLockElement !== canvas) {
      const [x, y] = canvasPoint(e);
      cursor(x, y);
    }
    if (e.movementX || e.movementY) mouseMotion(e.movementX, e.movementY);
  });
  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      // About one per wheel notch, as desktop mice report: browsers send
      // pixels (≈100 a notch), lines (≈3) or pages. +y is away from you.
      const per = e.deltaMode === 1 ? 1 / 3 : e.deltaMode === 2 ? 1 : 1 / 100;
      scroll(-e.deltaX * per, -e.deltaY * per);
      stats.input.wheel += 1;
    },
    { passive: false },
  );
  canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  // Touches, as pointer events (the mouse stays on the mouse events).
  const phases = { pointerdown: 0, pointermove: 1, pointerup: 2, pointercancel: 3 };
  for (const [type, phase] of Object.entries(phases)) {
    canvas.addEventListener(type, (e) => {
      if (e.pointerType !== "touch" && e.pointerType !== "pen") return;
      // No emulated mouse events after this.
      e.preventDefault();
      if (phase === 0) {
        canvas.focus();
        unlockAudio();
        stats.input.touches += 1;
      }
      const [x, y] = canvasPoint(e);
      touch(e.pointerId, phase, x, y);
    });
  }

  // Leaving the page lets go of everything held; coming back doesn't
  // count the time away as a frame.
  const letGo = () => {
    heldButtons.clear();
    heldModifiers = 0;
    focusLost();
    stats.input.focusLost += 1;
  };
  window.addEventListener("blur", letGo);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      letGo();
      if (sound.context && sound.context.state === "running") sound.context.suspend().catch(() => {});
    } else {
      resume();
      stats.input.resumed += 1;
      unlockAudio();
    }
  });

  // Pointer lock: the browser grants it only while the player has just
  // pressed something, so it's asked for then (and each frame while that
  // lasts), and given back as soon as the game lets go.
  let lockPending = false;
  const requestLock = () => {
    if (lockPending || document.pointerLockElement === canvas || !canvas.requestPointerLock) return;
    lockPending = true;
    stats.pointer.requests += 1;
    const done = () => (lockPending = false);
    const failed = () => {
      lockPending = false;
      stats.pointer.errors += 1;
    };
    try {
      const promise = canvas.requestPointerLock({ unadjustedMovement: true });
      if (promise && promise.then) {
        promise.then(done, () => {
          // No raw mouse here: plain lock.
          const retry = canvas.requestPointerLock();
          if (retry && retry.then) retry.then(done, failed);
          else done();
        });
      } else {
        done();
      }
    } catch {
      failed();
    }
  };
  function requestLockIfWanted() {
    if (stats.pointer.wanted) requestLock();
  }
  document.addEventListener("pointerlockchange", () => {
    lockPending = false;
    stats.pointer.locked = document.pointerLockElement === canvas;
  });
  document.addEventListener("pointerlockerror", () => {
    lockPending = false;
    stats.pointer.errors += 1;
  });
  const syncPointerLock = () => {
    const wanted = Boolean(core.web_cursor_locked());
    stats.pointer.wanted = wanted;
    const locked = document.pointerLockElement === canvas;
    if (wanted && !locked) {
      const active = navigator.userActivation ? navigator.userActivation.isActive : false;
      if (active) requestLock();
    } else if (!wanted && locked) {
      document.exitPointerLock();
    }
  };

  // Gamepads: polled each frame (the Gamepad API has no change events for
  // buttons), in the standard mapping.
  let padCount = 0;
  const pollGamepads = () => {
    if (!gamepads || !navigator.getGamepads) return;
    const pads = [];
    for (const pad of navigator.getGamepads()) {
      if (!pad || !pad.connected) continue;
      pads.push([
        pad.index,
        pad.id,
        Array.from(pad.buttons, (b) => Boolean(b.pressed || b.value > 0.5)),
        Array.from(pad.axes, Number),
      ]);
    }
    if (pads.length || padCount) {
      const list = pyodide.toPy(pads);
      try {
        gamepads(list);
      } finally {
        list.destroy();
      }
    }
    padCount = pads.length;
    stats.input.pads = padCount;
  };

  return { beforeFrame: pollGamepads, afterFrame: syncPointerLock };
}

// Full screen: browsers allow it only from a click, so the page has a
// button (F11 works too).
function installFullscreen() {
  const button = document.getElementById("fullscreen");
  const root = document.documentElement;
  if (!button || !root.requestFullscreen) return;
  button.hidden = false;
  button.addEventListener("click", () => {
    root.requestFullscreen({ navigationUI: "hide" }).catch(() => {});
    canvas.focus();
  });
}

// --- Start ------------------------------------------------------------

async function start() {
  const config = await (await fetch("player.json")).json();
  const pyodide = await loadPyodide({
    indexURL: new URL("pyodide/", location.href).href,
    stdout: (line) => console.log(line),
    stderr: (line) => console.warn(line),
  });
  mark("pyodide");
  stats.phase = "engine";
  status.textContent = "Loading the engine…";
  fitCanvas();
  pyodide.canvas.setCanvas3D(canvas);
  await pyodide.loadPackage(new URL(config.wheel, location.href).href);
  mark("engine");

  const core = pyodide.pyimport("automata._core");
  const audioReady = startAudio(core.web_sample_rate());

  stats.phase = "game";
  status.textContent = "Starting the game…";
  const source = await (await fetch(config.main)).text();
  pyodide.FS.writeFile(config.main, source);
  // Scripts run as on desktop; `game.run()` returns at once here.
  pyodide.runPython(`import runpy\nrunpy.run_path(${JSON.stringify(config.main)}, run_name="__main__")`);
  mark("game");
  await audioReady;

  const webFrame = core.web_frame;
  const input = installInput(pyodide, core);
  installFullscreen();
  stats.phase = "running";
  status.textContent = "";
  canvas.focus();
  const tick = () => {
    fitCanvas();
    let going;
    try {
      input.beforeFrame();
      going = webFrame(canvas.width, canvas.height);
      input.afterFrame();
      pumpAudio(core);
    } catch (error) {
      fail(error);
      return;
    }
    stats.frames += 1;
    if (stats.frames === 1) mark("first_frame");
    if (going) {
      requestAnimationFrame(tick);
    } else {
      stats.phase = "finished";
      if (document.pointerLockElement === canvas) document.exitPointerLock();
      if (sound.context) sound.context.close().catch(() => {});
    }
  };
  requestAnimationFrame(tick);
}

start().catch(fail);
