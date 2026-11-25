"""AUTOMATA on Android: the SDL3 spike.

A spinning cube on a floor, drawn by the engine through SDL's window, with
every finger on the screen marked. Back quits. CI runs this on an emulator,
sends it to the background and back, and reads the `automata-spike` lines
it prints (python-for-android sends them to logcat, tag `python`).
"""

import importlib

import automata as at

print(f"automata-spike start platform={getattr(at._core, 'platform', 'desktop')}")

# The standard library a game may use must survive the trimmed stdlib.zip
# (android/recipes/python3). CI waits for "stdlib ok".
STDLIB = """
    abc argparse array ast asyncio base64 bisect calendar collections colorsys
    concurrent.futures contextlib copy csv ctypes dataclasses datetime decimal
    difflib email.parser enum fnmatch fractions functools gettext glob gzip
    hashlib heapq hmac html http.client importlib.resources inspect io
    ipaddress itertools json locale logging math operator os pathlib
    pickle platform pprint queue random re secrets shlex shutil socket sqlite3
    ssl statistics string struct subprocess tarfile tempfile textwrap
    threading time tomllib traceback types typing unicodedata urllib.parse
    urllib.request uuid warnings weakref xml.etree.ElementTree zipfile zlib
    zoneinfo
""".split()
# Built only when their p4a recipes (libbz2, liblzma) are requested: reported.
OPTIONAL = ["bz2", "lzma"]


def import_failures(names):
    failures = []
    for name in names:
        try:
            importlib.import_module(name)
        except Exception as e:  # noqa: BLE001 - report every failure
            failures.append(f"{name} ({type(e).__name__}: {e})")
    return failures


missing = import_failures(STDLIB)
absent = import_failures(OPTIONAL)
print(f"automata-spike stdlib optional: {'; '.join(absent) or 'all present'}")
if missing:
    print(f"automata-spike stdlib missing: {'; '.join(missing)}")
else:
    print(f"automata-spike stdlib ok ({len(STDLIB)} modules)")

game = at.Game(title="AUTOMATA", target_fps=60, clear_color=(0.10, 0.12, 0.18, 1.0))
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

FINGER_COLOURS = [
    (0.30, 0.80, 1.00, 0.85),
    (1.00, 0.45, 0.55, 0.85),
    (0.55, 1.00, 0.45, 0.85),
    (1.00, 0.85, 0.30, 0.85),
]
state = {"touches_seen": 0, "spin": 0.0}


@game.update
def update(ctx):
    if ctx.input.just_pressed("back_button") or ctx.input.just_pressed("escape"):
        print("automata-spike back pressed: quitting")
        ctx.quit()
        return

    touches = ctx.input.touches
    # Spin faster while a finger is down: touch visibly drives the scene.
    state["spin"] += ctx.time.delta * (4.0 if touches else 1.2)
    ctx.scene["cube"].transform.rotation_euler = (0.25, state["spin"], 0.0)

    width, height = ctx.screen.size
    for touch in touches:
        if touch.just_started:
            state["touches_seen"] += 1
            x, y = touch.position
            print(f"automata-spike touch finger={touch.finger} at={x:.0f},{y:.0f}")
        x, y = touch.position
        colour = FINGER_COLOURS[touch.finger % len(FINGER_COLOURS)]
        ctx.screen.rect(x, y, 90.0, 90.0, colour)

    status = (
        f"AUTOMATA on Android   {width}x{height}   frame {ctx.time.frame}   "
        f"fingers {len(touches)}   touches {state['touches_seen']}"
    )
    ctx.screen.text(status, 24.0, 40.0, size=30.0, shadow=(0.0, 0.0, 0.0, 0.8))

    if ctx.time.frame % 60 == 0:
        print(f"automata-spike frame={ctx.time.frame} size={width}x{height}")


game.run()
print("automata-spike finished")
