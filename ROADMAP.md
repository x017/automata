# Roadmap

Every remaining feature, cut into phases that each ship on their own. Say
**"proceed"** and the next ⬜ phase in the [run order](#run-order) gets
built, tested, documented, pushed and reported. Then it's ticked ✅ here.

Each phase names the code it touches today, what "done" looks like, and a
size: **S** = about a day, **M** = a few days, **L** = a week or more.
Big multi-part designs that were already agreed live in
[`NEXT_PLANS.md`](NEXT_PLANS.md) (Android, game UI, skeletal animation);
the phases below link to them rather than repeat them.

Status: ✅ done · 🔨 in progress · ⬜ not started

## Every phase is done when

1. **Engine, Python and Oberon all have it** where it applies: the Rust
   crate, the scene file (`automata-core/src/scene_file.rs`, saved only
   when set, so old scenes stay byte-identical), Python
   (`automata-python/src/*.rs` + `_core.pyi`, or the shared
   `script_api.py` for script helpers, so Oberon Play and `run_scene`
   behave the same), and the Oberon Inspector / menus.
2. **Tests:** CPU unit tests; an offscreen GPU test for anything visible
   (`automata-renderer` `offscreen_*`, run on CI's software GPU); Python
   tests; Oberon UI tests for new rows and menus; the script conformance
   suite for new script API.
3. **Performance checked:** High and Beautiful frame times on the Lagoon
   and FPS views, before and after. Heavy effects stay off below their
   preset, following the agreed split: the sky, sun and nicer shadows also
   go on High; SSAO, refraction, lens flare, DoF and god rays are
   Beautiful only.
4. **Docs:** the wiki pages (`docs/wiki/...`), the API reference, and
   `scripts/build_site.py`.
5. **An example uses it** (Coin Run, FPS, Lagoon, or a new one) and its
   `build_level.py` regenerates the RON files.
6. **Gates:** `cargo fmt`, clippy `-D warnings` per crate, all test
   suites, CI green, then a report with before/after renders.

---

## Track A: Graphics

The "make it look like an indie game, not a toy" request. A1–A10 finish it;
A11–A13 are the big-world features after.

### A0. Sky, sun, lens flare, endless water ✅
### A0b. Materials: roughness, metal, normal maps, glow; Sphere primitive ✅
### A0c. Glowing objects light their surroundings ✅
### A4. Shadows from point lights (glowing objects) ✅
Done ahead of A1, at the user's request. High shadows the 2 nearest glow
lights, Ultra and Beautiful the 4 nearest (six-face shadow cubes, 384 /
512 px; soft taps on Beautiful). A "Glow Shadows" toggle is on meshes
(`glow_shadows=`, scene `Surface`, Oberon). Spotlight shadows move to A3,
together with spotlights.

**Performance fix (after A3).** A4–A3 had made every preset slower, Low
and Medium included, because they all ran one forward shader carrying all
the shadow code. The forward path now has one entry point per mix of
shadows a frame samples (none, sun, point lights, both), picked per frame:
Low and Medium never compile point shadow code, and High skips it when no
shadowed light is in view. Point shadow cube faces are cached and redrawn
only when their light or the casters they see change (still scenes redraw
none). The lookup is branch-free and matrix-free, skipped where a light is
too faint to show, and takes one bilinear compare on High (2x2 on Ultra,
8 taps on Beautiful). Software-GPU timings (1280×720, best run, ms):
Low back to its pre-A4 cost (Coin Run 315 before A4, 765 at A3, 340 now;
Lagoon 876 → 1350 → 876), Night Lights High 851 → 777, Ultra 1117 → 854,
Beautiful 1808 → 1432.

### A1. Shadows I: crisp sun shadows on High and Ultra ✅
Sun shadows are split into cascades: 2 on High and 3 on Ultra (2048 px each), plus
Beautiful's 4. Cascades cross-fade at their edges and fade out at the
shadow distance, and pull in to the level's size in small levels. The forward path uses
bilinear 4-tap filtering with a normal offset. Shaded sides lean toward
the sky colour when the sky dome is on. The sun has Shadow Distance and
Shadow Softness (scene, Python, Oberon Graphics tab and Inspector).
Measured cost: High +0–7%, Ultra within noise. (The 8-tap rotated Poisson
was too slow on High, at +15–24%.)

### A2. Shadows II: Beautiful's contact-hardening shadows and ambient occlusion ✅ M
Done. PCSS in `sample_cascade`: an 8-read blocker search (sunlit pixels
stop there), then 12 bilinear taps over a penumbra of tan(sun radius) ×
softness × caster distance, 1–16 texels. A depth prepass of the opaque
meshes into its own target (`ssao.rs`; the scene keeps its own depth, so
see-through meshes draw as before) feeds one half-resolution pass:
12-sample hemisphere AO over 1.5 m (normals from depth, squared) plus a
12-step, 0.45 m contact ray toward the sun, and the view depth. Two
depth-aware 7-tap blurs, then a depth-aware upsample in `fs_beautiful`.
AO dims ambient only; contact shadows multiply the sun shadow.
`Environment.ambient_occlusion` (0–2, default 1) in scene files, Python
and Oberon (Graphics tab and Inspector, "Occlusion"). Beautiful meshes
bind a separate light layout (binding 5), so High's pipelines are
untouched; a GPU test checks the crease darkens on Beautiful and High is
byte-identical at any strength. Measured cost: Coin Run about 2× faster
(the blocker search skips filtering in sun), FPS about +12%.

### A3. Placeable point lights and spotlights ✅ S
Done. `PointLight` and `SpotLight` scene components (colour, intensity,
range with 0 = auto, source radius, Cast Shadows; spots add cone and inner
angles). A spot shines along its entity's −Z. They work in scene files
(`point_light` / `spot_light`), Python (`at.PointLight`, `at.SpotLight`)
and Oberon: Create menu, Add Component, Inspector rows, light-coloured
viewport icons, and range rings or the cone out to its range on
selection. The renderer's `PointLight` gained `spot: Option<SpotCone>`.
`PointLightData` grew to 16 floats, carrying the axis and a cos-space
falloff scale/offset (a point light is scale 0, offset 1, so it is
unchanged). Shadow slots became a budget of six layers per shadowed point
light: a point light takes six cube faces, a spot one perspective view
over its cone plus 4°. So High's two slots shadow up to twelve spots.
Casters are culled by a cone test. A light inside a small mesh (bounding
radius up to 0.3 m plus the light's radius: a bulb, flame or lens) leaves
it out of its shadow. GPU test: a spot lights inside its cone, not
outside, and a blocker shades it on presets with point shadows. Demo:
`examples/oberon/night_lights` (a torch on a wall and two headlights, all
shadowed, with no light from a glowing mesh).

### A5. Water interaction I: ripples and wakes ✅ M
Done. `automata_physics::ripples::RippleField`: a 256² damped wave
equation, 0.25 m per cell (64 m across), with an edge sponge, scrolled in
whole cells to follow the view (centred 0.3 of its width ahead of the eye).
`apply_water` gives every dynamic **and kinematic** body crossing the
surface a dent (a cosine bump of its waterplane radius, as deep as its
draft); each step the field loses the new dent and gets the old one back,
so dropping in, bobbing and moving all make waves. Faster than the ripple
speed (1.6 m/s), moving makes a Mach-cone V (close to Kelvin's 19.5° at
boat speeds). Hitting the water faster than 1.5 m/s adds a splash. Foam
comes from how fast a dent moves the water and fades over a few seconds.
Calm water costs nothing; running, ~0.25 ms per step in release.
`PhysicsWorld3D::{splash, set_ripple_view, ripples, ripple_height}`; the
engine drivers and Oberon Play aim it at the drawn camera and hand a
`RippleMap` snapshot (RGBA half floats: height, slopes, foam) to
`DrawList::set_ripples`. The renderer uploads it to a new water binding
only when it changes. `water.wgsl` adds its height to the vertices and its
slopes (×3) to the normal, fading it at the field's edges, plus foam from
the field and on ripple crests, on every preset. Script API:
`ctx.water.splash(position_or_entity, strength)` and `ctx.water.height`
(shared API, so Oberon too, over the bridge as `splashes`), and
`ctx.physics.splash` for engine scripts. The Lagoon's crate now drops in
at Play start and its boat is kinematic, steered round the island by
`scripts/motor_boat.py`, riding the waves. `render_still` gained
`PLAY=seconds` to render a moment of Play. Tests: the ring speed and
symmetry, calming down, still dents making no waves, a V wake and calm
water ahead, the field following the view, a dropped crate's rings, a
moving body's wake, splashes (Rust, Python, the Oberon bridge), half-float
packing, and a GPU test that rings and foam change the water on Low and
Beautiful.
- Spec was: a ripple heightfield simulated around the camera, stamped by
  bodies crossing the surface; ripple normals and height in `water.wgsl`;
  V-shaped wakes and foam on crests; `ctx.water.splash(position,
  strength)`.
- **Done when:** dropping a crate in the Lagoon sends rings outward, and
  the boat leaves a wake. ✅

### A6. Particles ✅ L
Done. `automata_scene::ParticleEmitter` (28 fields: rate, burst,
duration, one-shot, lifetime, shape point/sphere/box/cone, cone angle,
speed, gravity, drag, size/colour over life with HDR colours, fades, spin,
stretch, blend, lit, soft, sprite or texture + sprite sheet, world or
local space) with six presets (Fire, Smoke, Sparks, Dust, Splash, Magic)
and `set_field` by name, shared by Python and the Oberon bridge.
`simulate_particles` runs every emitter on the CPU (a `ParticleState` per
entity; one-shot emitters despawn when done) in both engine drivers and
Oberon Play; `preview_particles` replays finished effects instead, for the
editor's live preview. `spawn_burst` / `burst` make one-off effects and
extra particles. Renderer (`particles.rs`, `particles.wgsl`): instanced
camera-facing quads, one draw per emitter, in their own pass after the
scene and water with the depth attached read-only, so they fade softly
where they meet it; alpha and additive pipelines; a procedural 5-sprite
sheet (dot, spark, star, flame, puff) or a loaded texture with frame
animation; lit particles take the sun, ambient and placed lights (per
particle, no shadows); stretched sparks along their velocity; HDR colours bloom on
Beautiful. Every preset draws them. Python: `at.ParticleEmitter(**fields)`,
`.preset(name, **fields)`, `.replace`, scene files, `ctx.scene.burst` /
`burst_emitter`; the shared script API has `ctx.particles.burst(effect,
position, direction, count, **fields)` and `ctx.particles.emit(entity,
n)`, over the bridge as `bursts` / `emits`. Oberon: Create → Particles,
Add Component → Particle Emitter, an Inspector section (preset, shape,
sprite and blend menus; glow fields keep HDR colours editable with the
colour picker), a flame icon, live preview while editing (particles carry
over edits). The FPS player's shots burst sparks along the hit normal
(hotter ones on drones); `examples/oberon/campfire` has fire, lit smoke,
embers and a crackle script. `render_still` gained `SHOOT` and `AIM`.
Tests: emission, bursts, one-shot cleanup, preview replay, shapes,
over-life curves, sprite sheets, fields by name, scene-file round trips,
GPU drawing (blend, soft edges, lit), Python, the bridge, the Inspector,
and the FPS example played in Oberon throwing sparks.
- Spec was: an instanced billboard pipeline with soft edges, additive or
  alpha blending, sprite-sheet animation and lit or unlit particles; a
  `ParticleEmitter` (rate, bursts, lifetime, shapes, velocity, gravity,
  drag, size/colour/alpha over life, world or local space) simulated on
  the CPU; Fire, Smoke, Sparks, Dust, Splash, Magic presets; Python
  `at.ParticleEmitter.preset("fire")` and `ctx` bursts; Oberon Inspector,
  live preview and Create → Particles.
- **Done when:** the FPS blaster throws sparks on hit, and a campfire
  example burns and smokes. ✅

### A7. Water interaction II: splashes, underwater, caustics ⬜ M
- Splash bursts (A6) sized by the entry speed, plus droplets and spray on
  wave crests in strong wind.
- Underwater view: fog and colour absorption when the camera is below the
  surface, a wobbling waterline, and muffled audio (a low-pass filter in
  `automata-audio`).
- Animated caustics projected onto geometry under the water.
- **Done when:** jumping into the Lagoon splashes, and diving shows a
  blue-green world with moving caustics.

### A8. A post chain for every preset, and colour grading ⬜ M
Today only Beautiful has HDR, bloom and FXAA (`post.rs`). The styles
need post effects on High too.
- A lighter LDR post chain on Medium and up: colour grading (exposure,
  contrast, saturation, temperature/tint, lift/gamma/gain), vignette,
  film grain and chromatic aberration; cheap bloom on High.
- `Environment` fields for each, plus a "Post" section in the Graphics
  tab.
- **Done when:** a desaturated, overexposed NieR grade and a warm,
  saturated BotW grade can be dialled in on High.

### A9. Atmosphere: height fog, god rays, depth of field, dust ⬜ M
- Height fog: thick in valleys, thin up high, lit by the sun.
- God rays on Beautiful: screen-space light shafts from the sun, blocked
  by geometry.
- Depth of field on Beautiful: the focus distance auto-follows the
  camera's target (or is set manually), with bokeh-like blur.
- Dust motes floating in sunbeams, a camera-attached particle preset
  (A6).
- **Done when:** the NieR checklist from planning (desaturated and
  overexposed grade, height fog, strong soft bloom, god rays, DoF, grain,
  chromatic aberration, vignette, dust) is all there.

### A10. Style presets: Realistic, Semi-realistic, BotW, A Short Hike, NieR ⬜ L
- Shading modes: **toon** (banded diffuse with a tint ramp and tinted
  shadows), rim light, and a painted-sky mode.
- **Outlines** (a depth/normal edge pass) and **pixelation** (render at
  low resolution, nearest-neighbour upscale, optional palette).
- A **Style** dropdown in the Graphics tab, stored on `Environment`. Each
  style sets environment, post and shading values. Styles mix: pick one,
  then override any knob, and the label shows "Custom".
- Python: `at.Environment.style("nier")`.
- **Done when:** the same Lagoon scene renders in all five styles, side
  by side in the docs.

### A11. Big worlds: culling, instancing, LOD ⬜ M
The main pass draws every mesh. Only shadows are culled today
(`mesh_spheres` in `renderer.rs`).
- Frustum culling in the main and shadow passes.
- Automatic instancing: same mesh and material → one draw call.
- Distance LOD: models can ship `_LOD1`/`_LOD2` meshes, or fade out past
  a distance.
- `examples/bench` gains a 10,000-tree forest.
- **Done when:** the forest runs at least 3× faster than today on High.

### A12. Grass and foliage (BotW-style fields) ⬜ L
Builds on A11's instancing.
- A `Grass` component: blades painted onto surfaces in Oberon with a
  brush (density, height, colour variation), drawn as instanced blades
  with distance fade.
- Wind (shared with water and trees), and blades that bend away from the
  player and objects.
- Foliage scatter: paint models (rocks, flowers, bushes) onto ground.
- **Done when:** a field of grass sways around the walking player with
  no frame drop on High.

### A13. Terrain ⬜ L
- A `Terrain` component: a heightmap with sculpt brushes in Oberon
  (raise, lower, smooth, flatten, noise) and 4-layer texture painting.
- A Rapier heightfield collider and water-level integration.
- Grass and foliage painting (A12) follow the terrain.
- **Done when:** an island with beaches, cliffs and grassy hills is
  sculpted entirely in Oberon.

---

## Track B: Animation and characters

The plan is [`NEXT_PLANS.md` #4](NEXT_PLANS.md#4-skeletal-animation--in-progress-phase-1-).
Import and GPU skinning are ✅.

### B1. Animation playback ⬜ L
NEXT_PLANS #4 phase 2:
- An `Animation` component (default clip, speed, play on start).
- `entity.animation.play("run", loop=True, fade=0.2)`, `.stop()`,
  `.playing`, `.time` in `script_api.py`.
- Crossfades.
- An Inspector clip list with a preview scrubber that plays in the
  editor.
- A CC0 animated character example.

### B2. Spring bones: hair, cloth, tails ⬜ M
- Verlet chains on chosen bones (stiffness, drag, gravity, wind), with
  sphere and capsule colliders on the body so hair doesn't pass through
  shoulders.
- Configured in the model's import settings (`.import.ron`), with an
  Oberon editor that picks the chain roots.
- **Done when:** a character's ponytail and skirt swing and settle as it
  runs and stops.

### B3. Animation events and root motion ⬜ M
NEXT_PLANS #4 phase 3:
- Per-clip events delivered to `on_animation_event`.
- Root motion driving the Character Controller.

### B4. Animation state machine and graph editor ⬜ L
NEXT_PLANS #4 phase 4:
- A `.anim.ron` graph with states, 1D blend spaces and conditional
  transitions.
- Scripts only set parameters.
- An Oberon node editor that highlights the live state during Play.

### B5. Attachments and IK ⬜ M
- Attach any entity to a bone (a sword in the hand, a hat on the head).
- Two-bone IK for feet on slopes and stairs and for hands reaching a
  target, plus look-at for heads and eyes.

### B6. Ragdolls ⬜ M
Needs C1 (joints).
- Generate capsule bodies and joints from the skeleton.
- Switch between animated and ragdoll, with blending back.

---

## Track C: Gameplay systems

### C1. Physics joints ⬜ M
Rapier's joint sets are already created and stepped in
`world2d_impl.rs` / `world3d_impl.rs`, but nothing exposes them.
- Fixed, hinge, ball, slider, rope and spring joints, with limits and
  motors.
- Scene components, Python and an Oberon target-entity picker.
- **Done when:** a swinging door, a rope bridge and a chain demo work.

### C2. Tweens and timers ⬜ S
- In `script_api.py`, so both runners get them:
  `ctx.tween(entity, "position", to, 0.5, ease="out_cubic")`,
  `ctx.after(2.0, fn)`, `ctx.every(1.0, fn)`, with cancel handles.
- 20 easing curves.

### C3. Save games and settings ⬜ S
- `ctx.save["coins"] = 12`, plus slots, stored as JSON in the platform's
  user-data folder, so it works in exports.
- Player settings (volume, graphics quality, key bindings) persisted
  automatically.

### C4. Navigation ⬜ L
- A navmesh baked from static colliders (voxelise → regions → polygons),
  with a "Bake" button and overlay in Oberon.
- `ctx.nav.path(a, b)` and a `NavAgent` component (speed, steering,
  avoidance between agents, off-mesh jump links).
- **Done when:** FPS enemies chase the player around obstacles.

### C5. AI helpers ⬜ S
- Pure-Python state-machine and behaviour-tree helpers in the script API.
- Perception: a sight cone with line-of-sight ray casts, and hearing
  radii.

### C6. 2D toolkit ⬜ L
The renderer draws sprites (`SpriteRenderer`), but Python and Oberon can
only draw them through `ctx.hud`.
- A sprite component in Python and Oberon, and a 2D scene mode in Oberon
  (orthographic, XY gizmos).
- Sprite sheets and frame animation, and a tilemap component with an
  Oberon tile painter (and auto-tiling).
- A 2D camera follow, plus 2D physics (already in the engine) in Oberon.
- **Done when:** a small platformer is built in Oberon without code
  except the player script.

---

## Track D: UI and editor

### D1. Game UI widgets ⬜ M
[`NEXT_PLANS.md` #3](NEXT_PLANS.md#3-game-ui-system--decided-phase-1--done)
phase 2: `ctx.ui.button/label/progress/panel/toggle/slider`, with
hover/press states and clicks consumed by the UI.

### D2. Custom fonts ⬜ S
#3 phase 3: `.ttf`/`.otf` files from the project, shown in Assets and
checked on export.

### D3. UI layout, focus and screens ⬜ M
#3 phase 4:
- Anchors and resolution scaling.
- Rows, columns and stacks.
- Gamepad and keyboard focus.
- Menu, pause and HUD screens.

### D4. UI editor in Oberon ⬜ L
#3 phase 5: `hud.ui.ron` documents composed on a canvas, with themes;
scripts bind to them by name.

### D5. Material assets and editor ⬜ M
- `.mat.ron` material files shared across entities, with texture slots
  (base, normal, roughness/metal, emissive) for primitives too. Today
  only models bring textures.
- An Oberon material panel with a live sphere preview; drag a material
  onto an object.

### D6. Editor workflow ⬜ M
- Multi-select editing (change a field on 20 entities at once).
- Prefab variants and per-instance overrides.
- A profiler panel (scripts / physics / render per frame).
- Snapping and duplicate-along-a-line.

### D7. Localization ⬜ S
- String tables (CSV) with `ctx.tr("menu.play")`, a language setting, and
  fonts falling back per language.

---

## Track E: Platforms

### E1. Android export ⬜ L
The phases in [`NEXT_PLANS.md` #1](NEXT_PLANS.md#1-android-export--decided-not-started)
(CPython on device, a template like Windows/Linux, emulator CI).

### E2. macOS export ⬜ M
- A macOS template build in CI (Apple Silicon and Intel) and an `.app`
  bundle from the Export dialog.
- Signing and notarisation are documented.

### E3. Web export ⬜ L
- wasm + WebGPU (WebGL2 fallback); Python through Pyodide.
- The open question in NEXT_PLANS #1 (size and start-up time) gets a
  prototype first.

### E4. Steam integration ⬜ M
- Achievements, stats, Cloud saves (on top of C3), the overlay, and
  Steam Deck input glyphs.

---

## Track F: Later, big and optional

### F1. Multiplayer ⬜ L
- Client–server with entity replication, prediction for the local
  player, and lobbies over UDP.
- This is a major design of its own and gets a planning round first.

### F2. Video playback ⬜ M
- Cutscenes and intro videos (a VP9/AV1 decoder) shown full screen or on
  a mesh.

### F3. Custom shaders ⬜ M
- User WGSL for materials and post effects, with hot reload, behind a
  safe interface (fixed inputs, validated on load).

Consoles aren't planned: they need platform-holder dev kits and NDAs.

---

## Run order

The order "proceed" follows. It finishes the graphics request first (as
agreed: sky → materials → shadows → water → styles), then the two "later"
projects the user asked for (spring bones, grass), then the rest by value
per effort. Reorder any time by telling me.

| # | Phase | Size | Status |
|---|---|---|---|
| 1 | A0 Sky, sun, flare, endless water | – | ✅ |
| 2 | A0b Materials | – | ✅ |
| 3 | A0c Glowing objects light surroundings | – | ✅ |
| 4 | A4 Point light shadows (glowing objects) | M | ✅ |
| 5 | A1 Crisp shadows on High/Ultra | M | ✅ |
| 6 | A2 Contact-hardening shadows + AO | M | ✅ |
| 7 | A3 Point lights and spotlights (+ spot shadows) | S | ✅ |
| 8 | A5 Water ripples and wakes | M | ✅ |
| 9 | A6 Particles | L | ✅ |
| 10 | A7 Splashes, underwater, caustics | M | ⬜ |
| 11 | A8 Post chain + colour grading | M | ⬜ |
| 12 | A9 Height fog, god rays, DoF, dust | M | ⬜ |
| 13 | A10 Style presets | L | ⬜ |
| 14 | B1 Animation playback | L | ⬜ |
| 15 | B2 Spring bones | M | ⬜ |
| 16 | A11 Culling, instancing, LOD | M | ⬜ |
| 17 | A12 Grass and foliage | L | ⬜ |
| 18 | C2 Tweens and timers | S | ⬜ |
| 19 | C3 Save games and settings | S | ⬜ |
| 20 | C1 Physics joints | M | ⬜ |
| 21 | D1 UI widgets | M | ⬜ |
| 22 | D2 Custom fonts | S | ⬜ |
| 23 | D3 UI layout, focus, screens | M | ⬜ |
| 24 | B3 Animation events, root motion | M | ⬜ |
| 25 | B4 Animation state machine | L | ⬜ |
| 26 | B5 Attachments and IK | M | ⬜ |
| 27 | C4 Navigation | L | ⬜ |
| 28 | C5 AI helpers | S | ⬜ |
| 29 | D5 Material assets and editor | M | ⬜ |
| 30 | A13 Terrain | L | ⬜ |
| 31 | B6 Ragdolls | M | ⬜ |
| 32 | C6 2D toolkit | L | ⬜ |
| 33 | D6 Editor workflow | M | ⬜ |
| 34 | D4 UI editor | L | ⬜ |
| 35 | D7 Localization | S | ⬜ |
| 36 | E1 Android export | L | ⬜ |
| 37 | E2 macOS export | M | ⬜ |
| 38 | E3 Web export | L | ⬜ |
| 39 | E4 Steam integration | M | ⬜ |
| 40 | F3 Custom shaders | M | ⬜ |
| 41 | F2 Video playback | M | ⬜ |
| 42 | F1 Multiplayer | L | ⬜ |

Large phases may span several "proceed"s. Each one ends with a pushed,
working step and a report on what's left in that phase.
