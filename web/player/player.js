// The AUTOMATA web player: loads Pyodide (bundled beside this file), the
// engine's wheel and the game, then runs one frame per animation frame.
//
// `player.json` names the wheel and the game's entry script. The game's
// `game.run()` only starts it (on the web the browser owns the loop); this
// file then calls `automata._core.web_frame(width, height)` each frame
// until it returns false. `window.automata` holds what CI reads: the
// phase, timings in milliseconds since navigation, frame count, errors.

import { loadPyodide } from "./pyodide/pyodide.mjs";

const canvas = document.getElementById("canvas");
const status = document.getElementById("status");
const stats = (window.automata = { phase: "loading", timings: {}, frames: 0, error: null });

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

  stats.phase = "game";
  status.textContent = "Starting the game…";
  const source = await (await fetch(config.main)).text();
  pyodide.FS.writeFile(config.main, source);
  // Scripts run as on desktop; `game.run()` returns at once here.
  pyodide.runPython(`import runpy\nrunpy.run_path(${JSON.stringify(config.main)}, run_name="__main__")`);
  mark("game");

  const webFrame = pyodide.pyimport("automata._core").web_frame;
  stats.phase = "running";
  status.textContent = "";
  canvas.focus();
  const tick = () => {
    fitCanvas();
    let going;
    try {
      going = webFrame(canvas.width, canvas.height);
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
    }
  };
  requestAnimationFrame(tick);
}

start().catch(fail);
