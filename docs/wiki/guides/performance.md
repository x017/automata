# Guide: Performance

How many scripted entities fit in a frame? This page answers that with
measured numbers, explains what the numbers include, and shows how to
measure on your own machine.

## Practical limits

At 60 FPS a frame lasts 16.7 ms, and that time is shared by your
scripts, physics and drawing. On the reference machine below:

- **Plain scripted entities** cost about **15 µs each** per frame in a
  game (`run_scene` or an exported build). About **1,000** fit in a 60 FPS
  frame before anything is drawn. Keep to **around 700** to leave room for
  rendering.
- **Scripted physics characters** (a character controller each) cost
  about **50 µs each**. Keep to **around 250**.
- **Oberon Play** takes 1.5 to 2 times as long per frame as the game,
  because every frame crosses to a separate Python process and back. A scene that runs at 30
  FPS in Play can run at 60 FPS once exported. Judge final performance
  with `run_scene` or an export, not in the editor.

Script time grows in proportion to the number of scripted entities, so you
can scale these figures to your own scene. A faster CPU than the reference
machine (a small cloud VM) will fit more.

## The numbers

Measured November 2026 on an Intel Xeon @ 2.60 GHz (2 threads, Linux 6.1),
with Python 3.11. Each run is headless, 300 frames after 30 warm-up frames.
Each frame advances exactly 1/60 s.

| Scene | Entities | Runner | Avg frame | p95 frame | Script | Engine | FPS |
|---|---:|---|---:|---:|---:|---:|---:|
| scripts | 100 | Oberon Play | 3.16 ms | 4.25 ms | 1.66 ms | 1.50 ms | 316 |
| scripts | 100 | `run_scene` | 1.55 ms | 2.26 ms | 1.40 ms | 0.16 ms | 644 |
| scripts | 100 | export | 1.54 ms | 2.22 ms | 1.39 ms | 0.15 ms | 649 |
| scripts | 1,000 | Oberon Play | 28.67 ms | 35.98 ms | 15.76 ms | 12.91 ms | 35 |
| scripts | 1,000 | `run_scene` | 16.52 ms | 20.05 ms | 14.94 ms | 1.58 ms | 61 |
| scripts | 1,000 | export | 15.77 ms | 18.22 ms | 14.32 ms | 1.45 ms | 63 |
| scripts | 5,000 | Oberon Play | 164.65 ms | 193.53 ms | 86.59 ms | 78.05 ms | 6 |
| scripts | 5,000 | `run_scene` | 112.07 ms | 132.22 ms | 100.96 ms | 11.11 ms | 9 |
| scripts | 5,000 | export | 116.03 ms | 137.03 ms | 104.62 ms | 11.41 ms | 9 |
| physics | 100 | Oberon Play | 7.06 ms | 9.27 ms | 2.58 ms | 4.48 ms | 142 |
| physics | 100 | `run_scene` | 3.89 ms | 4.87 ms | 1.52 ms | 2.36 ms | 257 |
| physics | 100 | export | 4.08 ms | 6.26 ms | 1.62 ms | 2.46 ms | 245 |
| physics | 1,000 | Oberon Play | 78.28 ms | 91.65 ms | 26.79 ms | 51.49 ms | 13 |
| physics | 1,000 | `run_scene` | 51.43 ms | 59.94 ms | 18.62 ms | 32.81 ms | 19 |
| physics | 1,000 | export | 49.47 ms | 56.50 ms | 17.60 ms | 31.87 ms | 20 |
| physics | 5,000 | Oberon Play | 406.34 ms | 443.43 ms | 142.08 ms | 264.26 ms | 2 |
| physics | 5,000 | `run_scene` | 273.74 ms | 308.76 ms | 110.41 ms | 163.34 ms | 4 |
| physics | 5,000 | export | 277.75 ms | 304.86 ms | 112.24 ms | 165.51 ms | 4 |

The **export** row used a development template, which runs the game with
the installed Python rather than a bundled one. That is why it matches
`run_scene` so closely.

## What is measured

The scenes come from `examples/bench/build_scenes.py`. Every entity runs
the same script, `examples/bench/bench.py`. Each frame it reads two
attributes, moves its entity in a small circle and casts a ray down at the
ground, which is what most gameplay scripts do.

- **scripts** scenes: cubes that move by writing their transform.
- **physics** scenes: capsules with a character controller, moved through
  `entity.character_velocity`, so physics work grows with the entity count
  too.

The columns:

- **Avg frame / p95 frame:** the average frame, and the frame that 95% of
  frames beat. The p95 figure shows how bad the occasional slow frame gets.
- **Script:** time spent running every entity's `update`.
- **Engine:** everything else in the frame. That covers the physics step,
  character controllers, syncing the world, and in Oberon Play the trip
  to the script process and back.
- **FPS:** 1000 divided by the average frame time.

The runs are **headless**, so drawing isn't included. Rendering cost
depends on your GPU, resolution and graphics quality rather than on the
script count. Leave part of the frame for it.

Every runner advances the game by exactly 1/60 s per frame (the
`fixed_delta` option of `at.Game`). Physics therefore takes one step per
frame however long the frame took, and each row shows the cost of one
60 Hz frame. A real game that falls below 60 FPS runs extra physics steps
to catch up, up to four per frame, so it slows down faster than these
rows suggest.

## Measure on your machine

```sh
python examples/bench/run.py            # every scene in every runner
python examples/bench/run.py --quick    # 100 and 1,000 entities only
python examples/bench/run.py --runners run_scene --sizes 100,1000,5000
```

Run it with the Python that has `automata` installed. It builds the
release binaries with cargo (`oberon`, `automata-export` and
`automata-player`), writes the scenes to `examples/bench/scenes/`, runs
each one and prints the table above with your machine and the date.
`--out results.json` also saves every frame time. A run that exceeds
`--seconds` (default 30) of measuring stops early and reports the frames
it managed.

For the export runner it uses an installed export template if there is
one, otherwise a development template made from the freshly built player.

The runners can also be used on their own:

- **Oberon Play:** `oberon --bench scene.ron --frames 300 --out stats.json`
  runs a scene the way Play mode does, without a window.
- **`run_scene` and exported games:** set `AUTOMATA_FRAME_STATS=stats.json`
  and the run writes every frame's time and script time when it ends.
  `AUTOMATA_FRAME_STATS_WARMUP` sets the frames to skip (default 30).
  An exported game takes `--headless --frames N --fixed-delta 0.0166667`.

**Rerun the benchmark before each release** and update this page, so that
a slowdown shows up as a change in the table.
