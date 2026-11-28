"""AUTOMATA in the browser: the web test game (roadmap E3).

A cube on a floor, drawn by the engine with WebGL2 inside Pyodide, that
answers every kind of input the web player passes on:

- keys (WASD / arrows) and a gamepad's left stick move the cube;
- a click or a tap changes the colour of the marker in the corner (and
  puts a dot where it landed), the wheel zooms the camera;
- Space (or the pad's south button) plays a beep, L locks the pointer
  (mouse movement then turns the cube), G switches between the High and
  Beautiful graphics presets.

CI (web/ci/smoke.mjs) loads it in headless Chromium, drives it with fake
keys, mouse, touches and a fake gamepad, and reads what the game saw from
`window.automata.game` (a JSON string written each frame) and the
`automata-spike` lines it prints.
"""

import json
import math
import struct
import wave

import automata as at

try:  # Pyodide's bridge to the page; absent on desktop.
    import js
except ImportError:
    js = None

print(f"automata-spike start platform={getattr(at._core, 'platform', 'desktop')}")

BEEP = "/tmp/automata-beep.wav"


def write_beep(path, seconds=0.25, freq=660.0, rate=44_100):
    """A short sine beep with a fade out, as 16-bit mono WAV."""
    frames = bytearray()
    count = int(seconds * rate)
    for i in range(count):
        fade = 1.0 - i / count
        sample = math.sin(2.0 * math.pi * freq * i / rate) * 0.8 * fade
        frames += struct.pack("<h", int(sample * 32767))
    with wave.open(path, "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes(bytes(frames))


write_beep(BEEP)

COLOURS = [
    (0.95, 0.55, 0.20, 1.0),
    (0.25, 0.75, 0.95, 1.0),
    (0.55, 0.90, 0.35, 1.0),
    (0.90, 0.35, 0.65, 1.0),
]

game = at.Game(title="AUTOMATA", clear_color=(0.10, 0.12, 0.18, 1.0))
game.load_scene(
    at.Scene()
    .add(at.Entity("camera").add(at.Camera3D(position=(0.0, 2.6, 6.0), target=(0.0, 0.6, 0.0))))
    .add(at.Entity("sun").add(at.DirectionalLight(direction=(-0.4, -1.0, -0.35), ambient=0.35)))
    .add(
        at.Entity("floor")
        .add(at.Transform(position=(0.0, -0.5, 0.0), scale=(12.0, 1.0, 12.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.30, 0.33, 0.40, 1.0)))
    )
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0.0, 0.8, 0.0)))
        .add(at.MeshRenderer.cube(size=1.2, color=(0.95, 0.55, 0.20, 1.0)))
    )
)

# What the game has seen, for CI (and the HUD).
seen = {
    "frame": 0,
    "size": [0, 0],
    "x": 0.0,
    "z": 0.0,
    "spin": 0.0,
    "zoom": 6.0,
    "keys": [],
    "clicks": 0,
    "cursor": None,
    "scroll": 0.0,
    "mouse_motion": 0.0,
    "touches_started": 0,
    "touch_at": None,
    "tap_at": None,
    "pad": None,
    "pad_events": [],
    "beeps": 0,
    "cursor_locked": False,
    "shift": False,
    "graphics": "",
    "graphics_frames": 0,
}
seen_keys = set()
held = {"south": False}


def tap(at_point):
    seen["clicks"] += 1
    seen["tap_at"] = at_point


@game.update
def update(ctx):
    dt = ctx.time.delta
    inp = ctx.input
    pads = inp.gamepads

    # Movement: keys and the left stick.
    move_x = (inp.pressed("d") or inp.pressed("arrow_right")) - (inp.pressed("a") or inp.pressed("arrow_left"))
    move_z = (inp.pressed("s") or inp.pressed("arrow_down")) - (inp.pressed("w") or inp.pressed("arrow_up"))
    pad = pads.primary()
    if pad is not None:
        stick_x, stick_y = pad.left_stick()
        move_x += stick_x
        move_z -= stick_y  # +y is up on the stick, away from the camera
        seen["pad"] = {
            "name": pad.name,
            "south": pad.pressed("south"),
            "stick": [round(stick_x, 2), round(stick_y, 2)],
        }
    else:
        seen["pad"] = None
    for event in pads.events:
        label = event.kind if event.button is None else f"{event.kind}:{event.button}"
        if label not in seen["pad_events"]:
            seen["pad_events"].append(label)
    seen["x"] = max(-5.0, min(5.0, seen["x"] + move_x * 3.0 * dt))
    seen["z"] = max(-5.0, min(5.0, seen["z"] + move_z * 3.0 * dt))

    for key in ("w", "a", "s", "d", "arrow_left", "arrow_right", "space", "l", "g", "shift_left"):
        if inp.just_pressed(key) and key not in seen_keys:
            seen_keys.add(key)
            seen["keys"] = sorted(seen_keys)
    seen["shift"] = inp.modifiers.shift

    # Mouse: clicks recolour, the wheel zooms, locked movement turns.
    if inp.mouse_just_pressed("left"):
        tap(inp.cursor)
    # (A held left button counts as a finger too, and a finger presses
    # the left button: both ways round, game code needn't care which.)
    for touch in inp.touches:
        if touch.just_started:
            seen["touches_started"] += 1
            seen["touch_at"] = [round(touch.position[0]), round(touch.position[1])]
    if inp.cursor is not None:
        seen["cursor"] = [round(inp.cursor[0]), round(inp.cursor[1])]
    seen["scroll"] += inp.scroll[1]
    seen["zoom"] = max(3.0, min(12.0, 6.0 - seen["scroll"] * 0.5))
    dx, dy = inp.mouse_delta
    seen["mouse_motion"] += abs(dx) + abs(dy)
    if ctx.screen.cursor_locked:
        seen["spin"] += dx * 0.01

    # Space or the south button: a beep. L: lock the pointer (or free it).
    south = pad is not None and pad.pressed("south")
    pressed_south = south and not held["south"]
    held["south"] = south
    if inp.just_pressed("space") or pressed_south:
        ctx.audio.play(BEEP, volume=0.8)
        seen["beeps"] += 1
    if inp.just_pressed("l"):
        ctx.screen.lock_cursor(not ctx.screen.cursor_locked)
    seen["cursor_locked"] = ctx.screen.cursor_locked
    if inp.just_pressed("g"):
        ctx.screen.set_graphics("high" if ctx.screen.graphics == "beautiful" else "beautiful")
    graphics = ctx.screen.graphics
    if graphics != seen["graphics"]:
        seen["graphics"] = graphics
        seen["graphics_frames"] = 0
    seen["graphics_frames"] += 1  # frames run on this preset

    seen["spin"] += dt * 1.2
    cube = ctx.scene["cube"].transform
    cube.position = (seen["x"], 0.8, seen["z"])
    cube.rotation_euler = (0.25, seen["spin"], 0.0)
    ctx.scene["camera"].camera_3d = at.Camera3D(
        position=(0.0, 2.6 * seen["zoom"] / 6.0, seen["zoom"]), target=(0.0, 0.6, 0.0)
    )

    width, height = ctx.screen.size
    seen["frame"] = ctx.time.frame
    seen["size"] = [width, height]
    shadow = (0.0, 0.0, 0.0, 0.8)
    ctx.screen.text(
        f"AUTOMATA on the web   {width}x{height}   frame {ctx.time.frame}",
        24.0, 40.0, size=30.0, shadow=shadow,
    )
    pad_text = seen["pad"]["name"][:28] if seen["pad"] else "none"
    ctx.screen.text(
        f"keys {' '.join(seen['keys']) or '-'}   clicks {seen['clicks']}   "
        f"touches {seen['touches_started']}   pad {pad_text}   beeps {seen['beeps']}",
        24.0, 76.0, size=18.0, shadow=shadow,
    )
    ctx.screen.text(
        f"WASD / arrows / stick: move   click, tap: colour   wheel: zoom   Space: beep   "
        f"L: lock pointer   G: graphics ({seen['graphics']})",
        24.0, height - 28.0, size=16.0, color=(0.75, 0.8, 0.9, 1.0), shadow=shadow,
    )
    marker = COLOURS[seen["clicks"] % len(COLOURS)]
    ctx.screen.rect(width - 64.0, 24.0, 40.0, 40.0, color=marker)
    if seen["tap_at"] is not None:
        tx, ty = seen["tap_at"]
        ctx.screen.rect(tx - 6.0, ty - 6.0, 12.0, 12.0, color=marker)
    if js is not None:
        js.window.automata.game = json.dumps(seen)
    if ctx.time.frame % 60 == 0:
        print(f"automata-spike frame={ctx.time.frame} size={width}x{height}")


game.run()
print(f"automata-spike started stepping={game.stepping}")
