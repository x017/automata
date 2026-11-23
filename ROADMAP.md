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

### A7. Water interaction II: splashes, underwater, caustics ✅ M
Done: bodies and characters that hit the water faster than 1.5 m/s throw a
splash burst sized by their entry speed, and `Water.wind` above 0.5 blows
spray off the crests near the camera. With the camera under the surface,
every preset fogs and absorbs colour toward blue-green, shows the surface
from below (with total internal reflection) and a wobbling waterline, and
the main audio track goes through a 650 Hz low-pass (engine and Oberon
Play; `ctx.audio.set_low_pass` for scripts). Medium and up draw animated
caustics (`Water.caustics`) on whatever is under the nearest four waters.
The Lagoon has a swimmer on the pier (Tab, run, jump, C to dive) and the
orbit camera follows it under.
- Spec was: splash bursts (A6) sized by the entry speed, plus droplets and
  spray on wave crests in strong wind; an underwater view with fog and
  colour absorption, a wobbling waterline and muffled audio (a low-pass
  filter in `automata-audio`); animated caustics projected onto geometry
  under the water.
- **Done when:** jumping into the Lagoon splashes, and diving shows a
  blue-green world with moving caustics. ✅

### A8. A post chain for every preset, and colour grading ✅ M
Today only Beautiful has HDR, bloom and FXAA (`post.rs`). The styles
need post effects on High too.
- A lighter LDR post chain on Medium and up: colour grading (exposure,
  contrast, saturation, temperature/tint, lift/gamma/gain), vignette,
  film grain and chromatic aberration; cheap bloom on High.
- `Environment` fields for each, plus a "Post" section in the Graphics
  tab.
- **Done when:** a desaturated, overexposed NieR grade and a warm,
  saturated BotW grade can be dialled in on High. ✅

Done: `Environment` grades the frame after tone mapping on Medium and up
(white balance, lift/gamma/gain, contrast, saturation, vignette, grain,
chromatic aberration) and High/Ultra get a cheap quarter-resolution bloom;
Beautiful folds the same grade into its HDR resolve. Five looks
(`PostEffects::PRESETS`: Neutral, Bleached, Vivid, Film, Noir) in Python
(`post="bleached"`, `with_post`) and in Oberon's new Post group. A neutral
grade with no bloom skips the pass entirely.

### A9. Atmosphere: height fog, god rays, depth of field, dust ✅ M
- Height fog: thick in valleys, thin up high, lit by the sun.
- God rays on Beautiful: screen-space light shafts from the sun, blocked
  by geometry.
- Depth of field on Beautiful: the focus distance auto-follows the
  camera's target (or is set manually), with bokeh-like blur.
- Dust motes floating in sunbeams, a camera-attached particle preset
  (A6).
- **Done when:** the NieR checklist from planning (desaturated and
  overexposed grade, height fog, strong soft bloom, god rays, DoF, grain,
  chromatic aberration, vignette, dust) is all there. ✅

Done: `Environment` gained `fog_height`/`fog_falloff` (fog integrated
along each view ray through an exponential height layer, every preset)
and `fog_sun`; on Beautiful, `god_rays` (a quarter-resolution radial blur
of the sky around the sun, masked by depth so geometry casts shafts) and
`depth_of_field` (a half-resolution disc blur whose circle of confusion
follows `focus_distance`, or the camera's target, with a depth-aware
gather and upsample so in-focus edges don't halo). The `"motes"` particle
preset fills a box around the camera (`follow_camera`, particles wrap),
and lit particles take the sun's shadow. Daylight and dusk turn on height
fog and god rays; the lagoon example wears the full list (Film look plus
all four). Python keywords, the stub, and Oberon rows for all of it.

### A10. Style presets: Realistic, Semi-realistic, BotW, A Short Hike, NieR ✅ L
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

Done: `Environment` gained toon bands (with softness, flat highlights),
a shade tint, rim light and a painted sky, shared by the forward and
physically based shaders so they work on every preset. Outlines come from
a second difference of 1/depth (silhouettes and creases, drawn before
water so it covers them). Pixels and a dithered palette run in the grade
pass (Medium to Ultra) or FXAA's (Beautiful), sized for a 720 px screen.
`Environment::STYLES` and `with_style` set five looks while keeping the
scene's colours and sky, and `style_name` reads "Custom" once a knob
changes. Python has `Environment.style("nier")`, `with_style`, `styles()`
and `style_name`. Oberon has a Style group (dropdown plus every knob) in
the Graphics tab and the Inspector. `docs/assets/rendering/styles.jpg`
shows the Lagoon in all five.

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

### B1. Animation playback ✅ L
NEXT_PLANS #4 phase 2:
- An `Animation` component (default clip, speed, play on start).
- `entity.animation.play("run", loop=True, fade=0.2)`, `.stop()`,
  `.playing`, `.time` in `script_api.py`.
- Crossfades.
- An Inspector clip list with a preview scrubber that plays in the
  editor.
- A CC0 animated character example.

Done: `automata_scene::Animation { clip, speed, looping, play_on_start }`
is saved in scene files, and an `Animator` plays it with smoothstep
crossfades (a fade interrupted mid-way blends on from the pose shown).
Scripts get `entity.animation.play(clip, loop, fade, speed, restart)`,
`.stop(fade)`, `.clip`, `.time` (settable), `.duration`, `.playing`,
`.speed`, `.clips` and `.length(clip)`. An unknown clip raises `KeyError`,
and asking for the playing clip again keeps it going, so scripts can pick
a clip every frame. Python games get the same as `view.animation`, plus
`at.Animation`. Oberon gives a dropped-in animated model an Animation set
to its idle clip, holds each model on its clip's first frame while
editing, and the Inspector's Animation section has a clip menu, Speed,
Loop, Play On Start and a Preview row (play/pause and a scrubbing time
bar) that animates in the scene view without pressing Play. Newly
materialized models keep their animator's pose instead of flashing the
rest pose. `examples/oberon/robot_plaza` uses Quaternius' CC0
RobotExpressive: a player script that blends Idle, Walking, Running, Jump
and Wave, a greeter that waves when you come near, and robots that dance,
sit (a one-shot holding its last frame) and walk in slow motion. Docs:
[Models → Animation](docs/wiki/oberon/models.md#animation).

### B2. Spring bones: hair, cloth, tails ✅ M
- Verlet chains on chosen bones (stiffness, drag, gravity, wind), with
  sphere and capsule colliders on the body so hair doesn't pass through
  shoulders.
- Configured in the model's import settings (`.import.ron`), with an
  Oberon editor that picks the chain roots.
- **Done when:** a character's ponytail and skirt swing and settle as it
  runs and stops.

Done: `ImportSettings.springs` holds chains (root bone, stiffness, drag,
gravity, wind, thickness) and sphere/capsule colliders (bone, offset,
radius, length), saved in `<model>.import.ron` and copied into the rig.
`automata_scene::SpringSim` simulates each swinging bone's tip with Verlet
in world space (substepped, the entity's move spread across substeps),
springs it towards its animated place, keeps the bone's length, projects it
out of colliders onto the reachable circle, and turns the bone to follow.
Forks (a skirt's waistband) stay put while their branches swing; teleports
reset. Springs damp in proportion to their rate so long chains settle
instead of whipping. The `Animator` runs it after the clip with the
entity's `GlobalTransform`, and models with springs but no clips get one.
Scripts get `entity.animation.springs` and `.reset_springs()`. Oberon's
Import block has a Springs section (+ Chain, + Collider, per-chain and
per-collider numbers, × to remove), with bones picked from a searchable
palette that lists likely roots first. `examples/oberon/ponytail_park` has
a generated CC0 runner with a ponytail and an eight-panel skirt; a Rust
test runs it, stops it and checks the hair trails, swings forward, stays
out of the head and settles. Docs:
[Models → Spring bones](docs/wiki/oberon/models.md#spring-bones).

### B2b. Clips from other files ✅ S
- A model borrows the animations of other glTF files made for the same
  skeleton (one file per move, as animation packs and Mixamo ship them).
- **Done when:** a clip from another file plays on the model like its own.

Done: `ImportSettings.clips` lists the files (relative to the model, saved
in `<model>.import.ron`). Loading the model loads each file's clips and
retargets them onto the model's rig by bone name (`Clip::retarget`), so the
node order may differ and keys for missing bones are dropped. A file with
one clip names it after the file (`run.glb` gives `run`); a file with
several gives `file:Clip` (`sword:Slash`). Names stay exact, and a name the
model already has keeps the model's clip, with a warning. Missing or broken
files are skipped with a warning but still watched, so hot reload and
`automata_assets::dependencies` (Oberon's reload, the export check) cover
them. Oberon's Import block has a Clip files section (+ Clips from file…,
a palette of the project's other glTF files; each file with the clips it
gave, red when it gave none; × to remove). Ponytail Park's emotes (1 waves,
2 cheers) come from `models/anims/emotes.glb`, a skeleton-only file from
`make_runner.py`. Docs:
[Models → Clips from other files](docs/wiki/oberon/models.md#clips-from-other-files).

### B3. Animation events and root motion ✅ M
NEXT_PLANS #4 phase 3. Events are named moments in clips, marked under
`events` in `<model>.import.ron` (`(clip: "Run", time: 0.18, name:
"footstep")`, borrowed clips included). They're stored on the `Clip`
(sorted, clamped). The `Animator` collects the events its playhead passes
each step: forwards, backwards, across loop wraps, and at exactly the
start of a fresh play. A one-shot clip fires `end`, and a clip fading out
fires nothing. They go into a ring of the latest 32 with a per-entity
`seq`, carried to scripts in `AnimationState.events` (bridge `ev`), so
skipped bridge frames lose nothing. `script_api` delivers each once,
at the start of the next frame, to `on_animation_event(entity, ctx, name)`
and to the channel `<entity>:animation:<event>`. The new channels API is
`ctx.subscribe(pattern, fn)` with `*`/`?` wildcards, plus `unsubscribe`,
`publish` and `Subscription.cancel()`. Re-subscribing is a no-op,
subscribers follow hot reloads, errors report against the subscriber's
script, and subscriptions end with their entity.

Root motion (`Animation.root_motion`, the Inspector's Root Motion row,
`entity.animation.root_motion`) uses `ClipMotion`: the shallowest bone
with a translation channel, measured in model space. Its ground (XZ)
travel is pinned out of the pose and moves the entity instead, so a jump's
height stays in the animation. Characters get a `CharacterMotion` that
`PhysicsWorld3D::move_characters` consumes on top of their velocity, so
walls stop them. Other entities have their transform moved directly. The
Preview bar shows event ticks. Ponytail Park: footstep dust from one
`*:animation:footstep` subscription, and a 3-key Leap (2.4 m, borrowed
from `emotes.glb`) by root motion. Play sessions now pin model paths to the
document's folder. Docs:
[Models → Animation events](docs/wiki/oberon/models.md#animation-events),
[Models → Root motion](docs/wiki/oberon/models.md#root-motion).

### B3b. Key store for scripts ✅ M
Engine state by key, Redis style, in `script_api.KeyStore` (Oberon's Play
and `run_scene` alike): `ctx.get` / `ctx.set` / `ctx.scan` / `ctx.keys` /
`ctx.ref`. Keys are `entity:component:field`. The components are
`animation` (clip, time, speed and root_motion settable; setting `clip`
plays it), `transform` (position, rotation, scale) and `attr`. Shorter keys
give the component handle or the entity. `get`/`set`/`ref` take one exact
key: a shared name is an error that points to `scan`, and wildcards are
refused. `scan`/`keys` take `*`/`?`, keep scene order and skip entities
without the component or field. Wrong entity, component, field or attribute
names raise `KeyError` with a difflib "did you mean". Read-only fields raise
`AttributeError`, and `set` won't create attributes. `ctx.ref` returns a
`ComponentRef` / `FieldRef` that re-resolves on each use and raises once
its entity is gone. Colons are refused in entity names (`at.Entity`,
`Entity.name`, Oberon's rename). Oberon's Inspector notes shared names
and names with `:` under the entity name. Ponytail Park: C makes the
Spectator cheer through `ctx.ref("Spectator:animation")`. Docs:
[Scripting → Keys](docs/wiki/oberon/scripting.md#keys),
[Channels](docs/wiki/oberon/scripting.md#channels).
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

### D0. The `oberon` command: CLI, new projects, a bundled Python ⬜ M
One `oberon` binary for everything, and new projects that work in any IDE
from the first minute, like an engine that ships its own SDK. Today
`oberon` takes a scene path or `--bench`, uses whatever `python3` is on
the PATH and finds a language server only if one is installed.
- **Subcommands:**
  - `oberon` / `oberon open [scene|folder]` opens the editor (a project
    picker with no argument).
  - `oberon new <name> [--template 3d|2d|empty]` makes a project.
  - `oberon run [scene]` plays a scene in a window without the editor.
  - `oberon test` runs the project's tests headless in its venv.
  - `oberon export --target windows|linux|android …` wraps automata-export.
  - `oberon bench` (today's `--bench`).
  - `oberon python …` / `oberon pip …` run the project's own interpreter.
  - `oberon doctor` checks the GPU, Python, the venv and the language server.
  - `--help` on each, with an exit code and messages a CI log can use.
- **The template:** `oberon new` writes `level.ron`, `scripts/` (a typed
  starter script), `models/`, `sounds/`, `tests/test_level.py`, `README.md`,
  `.gitignore`, a `pyproject.toml` pinning `automata-engine` to the
  editor's version, `pyrightconfig.json` and `.vscode/settings.json`
  pointing at `.venv`.
- **A bundled Python:** the Oberon download carries a standalone CPython
  (python-build-standalone) and the matching `automata-engine` wheel.
  `oberon new` makes `.venv` from them offline, with `uv` if it's there and
  `python -m venv` if not, so a machine with no Python works. Opening an
  older project offers to upgrade its wheel to the editor's version.
- **Typing from the first line:** the wheel ships `_core.pyi`, `py.typed`
  and typed `Entity` / `Context` / `Message` / `ComponentRef` classes for
  behaviour scripts. Templates write
  `def update(entity: Entity, ctx: Context)`, so VS Code, PyCharm and
  Oberon's own editor complete `ctx.get(...)`, `entity.animation.play(...)`
  and the rest, and flag typos. basedpyright goes into the venv as a dev
  dependency, so Oberon's code editor has a language server with nothing
  to install.
- **One interpreter:** Play, `oberon run`, `oberon test` and the language
  server all use the project's `.venv`. The preferences' Python path stays
  as an override.
- **Done when:** on a clean Windows and Linux machine with no Python, unzip
  Oberon, run `oberon new mygame`, then `code mygame`. `ctx.` completes
  with docs and a misspelt `entity.animaton` is underlined. `oberon open`,
  Play, `oberon run` and `oberon test` all work, and the same steps pass in
  CI.

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
(CPython on device, a template like Windows/Linux, emulator CI). The
template is built in CI with python-for-android's **SDL3 launcher**. The
engine gets an internal SDL3 driver for Android next to winit on desktop,
and its own input types. The game-facing API stays the same everywhere.

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
| 10 | A7 Splashes, underwater, caustics | M | ✅ |
| 11 | A8 Post chain + colour grading | M | ✅ |
| 12 | A9 Height fog, god rays, DoF, dust | M | ✅ |
| 13 | A10 Style presets | L | ✅ |
| 14 | B1 Animation playback | L | ✅ |
| 15 | B2 Spring bones | M | ✅ |
| 16 | B2b Clips from other files | S | ✅ |
| 17 | B3 Animation events, root motion | M | ✅ |
| 18 | B3b Key store for scripts | M | ✅ |
| 19 | A11 Culling, instancing, LOD | M | ⬜ |
| 20 | D0 The oberon command, new projects, bundled Python | M | ⬜ |
| 21 | A12 Grass and foliage | L | ⬜ |
| 22 | C2 Tweens and timers | S | ⬜ |
| 23 | C3 Save games and settings | S | ⬜ |
| 24 | C1 Physics joints | M | ⬜ |
| 25 | D1 UI widgets | M | ⬜ |
| 26 | D2 Custom fonts | S | ⬜ |
| 27 | D3 UI layout, focus, screens | M | ⬜ |
| 28 | B4 Animation state machine | L | ⬜ |
| 29 | B5 Attachments and IK | M | ⬜ |
| 30 | C4 Navigation | L | ⬜ |
| 31 | C5 AI helpers | S | ⬜ |
| 32 | D5 Material assets and editor | M | ⬜ |
| 33 | A13 Terrain | L | ⬜ |
| 34 | B6 Ragdolls | M | ⬜ |
| 35 | C6 2D toolkit | L | ⬜ |
| 36 | D6 Editor workflow | M | ⬜ |
| 37 | D4 UI editor | L | ⬜ |
| 38 | D7 Localization | S | ⬜ |
| 39 | E1 Android export | L | ⬜ |
| 40 | E2 macOS export | M | ⬜ |
| 41 | E3 Web export | L | ⬜ |
| 42 | E4 Steam integration | M | ⬜ |
| 43 | F3 Custom shaders | M | ⬜ |
| 44 | F2 Video playback | M | ⬜ |
| 45 | F1 Multiplayer | L | ⬜ |

Large phases may span several "proceed"s. Each one ends with a pushed,
working step and a report on what's left in that phase.
