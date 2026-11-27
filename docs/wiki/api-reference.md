# API Reference

Every public name in the `automata` package. Conventions: angles are
radians (`Transform`) or degrees (`Camera3D.fov`); colors are float tuples
in `0.0..=1.0`; `Vec3 = tuple[float, float, float]`.

```python
import automata as at
at.__version__          # engine version string
at.Error                # engine runtime failure exception
```

## Game

```python
at.Game(
    title="AUTOMATA",
    resolution=(1280, 720),
    headless=False,                     # no window/GPU; for tests and CI
    resizable=True,
    vsync=True,
    log_filter=None,                    # e.g. "debug"; RUST_LOG overrides
    target_fps=None,                    # int fps cap; None = engine default 60
    clear_color=None,                   # (r, g, b, a) window background
    graphics=None,                      # at.GraphicsQuality.HIGH etc.
    fixed_delta=None,                   # headless: every frame advances exactly this many seconds
)
```

| Member | Type | Notes |
|---|---|---|
| `run(update=None, max_frames=None) -> int` | method | drives the loop; returns frames run; a Game runs once |
| `load_scene(scene)` | method | replaces the world: at once before `run`, at the end of the frame inside `update` or a handler |
| `quit()` | method | requests shutdown |
| `stepping` | property | `True` while the browser drives the game frame by frame (web builds: `run` returns at once) |
| `on_finish(fn)` | method | calls `fn()` when a web game ends (at once elsewhere): cleanup that would follow `run` |
| `state` | property | `"created"` / `"running"` / `"shutdown"` |
| `update(fn)` | decorator | registers the per-frame callback |
| `on_event(name)` | decorator | registers a handler for events named `name` |

OOP subclass hooks: `on_start()`, `on_update(ctx)`, `on_<event>(payload)`.
Class attributes mirror every constructor option; kwargs win over them.

## Scene & Entity (staging)

```text
scene = at.Scene().add(entity)     # chainable
len(scene)  scene.entity_count

scene.to_ron() -> str              # staged scene -> RON text
scene.save(path)                   # write the RON scene file
at.Scene.from_ron(text) -> Scene   # ValueError on bad/newer files
at.Scene.load(path) -> Scene       # IOError on missing files

game.scene_to_ron() -> str         # the LIVE world (post-physics) -> RON
game.save_scene(path)              # same, straight to disk

entity = at.Entity(name=None)      # name optional, used for ctx.scene lookup and keys; no ":"
entity.add(component)              # chainable; TypeError for unknown types
entity.name                        # get/set
```

See [Guide: Scene Files](guides/scene-files.md) for the format and its
durability rules.

`at.Component` — the union of everything `add` accepts (for annotations).

### Transform

```text
at.Transform(position=(0, 0, 0), rotation_euler=(0, 0, 0), scale=(1, 1, 1))
t.position / t.rotation_euler / t.scale      # get/set (radians, XYZ order)
t.copy() -> Transform
```

### Cameras

```python
at.Camera3D(position=(0, 0, 0), target=(0, 0, -1), fov=60.0)   # fov in degrees
at.Camera2D(center=(0.0, 0.0), half_height=100.0)
```

Read-only properties: `position`, `target`, `fov` / `center`,
`half_height`. The first camera entity in the scene wins each frame;
replace per frame via `EntityView.camera_3d`.

### 2D: sprites, tilemaps, follow camera

```text
at.SpriteClip(name, first=0, last=None, fps=10.0, looping=True)   # last None: = first
at.Sprite(image="", size=(1, 1), color=(1, 1, 1, 1), flip_x=False, flip_y=False,
          pixelated=True, layer=0, columns=1, rows=1, frame=0, clips=[], play="")

at.AutoTile(name, first=0)         # 16 tiles from `first`, picked by neighbours
at.Tilemap(width=16, height=9, tileset="", columns=1, rows=1, tile_size=1.0,
           autotiles=[], color=(1, 1, 1, 1), pixelated=True, layer=0, collide=True)
at.Tilemap.from_rows(rows_text, legend, **same options)   # top row first
tm.get(x, y) -> int                # 0 empty, n tile n-1, -n auto-tile n-1
tm.tile_at(x, y) -> int | None     # the tile shown, auto-tiles resolved
tm.set(x, y, value) / tm.fill((x0, y0), (x1, y1), value) / tm.resize(w, h)
                                   # each returns a changed copy
tm.cell_at(x, y) -> (cx, cy)       # the cell under a local point
tm.width, tm.height, tm.filled     # filled: painted cells

at.CameraFollow2D(target="Player", offset=(0, 1), smoothing=0.15,
                  dead_zone=(0.5, 0.5), bounds=None)   # bounds: (left, bottom, right, top)
```

Cell (0, 0) is the tilemap's bottom-left, at its entity's position. A
`collide=True` map is solid in 2D physics. Put `CameraFollow2D` on the
entity with the `Camera2D`; it moves that entity's Transform. Scripts
drive sprites through `entity.sprite` (`play(clip, restart=False)`,
`stop()`, `clip`, `clips`, `finished`, `frame`, `flip_x`, `flip_y`,
`color`, `layer`). See [2D games](oberon/2d.md).

### Visuals

```python
at.MeshRenderer.cube(size=1.0, color=(1, 1, 1, 1))
at.MeshRenderer.sphere(size=1.0, color=(1, 1, 1, 1))  # size = diameter
at.MeshRenderer.quad(size=1.0, color=(1, 1, 1, 1))
at.MeshRenderer.model(path, color=(1, 1, 1, 1))   # .glb / .gltf / .obj
# every constructor also takes the keyword-only surface arguments:
#   roughness=None, metallic=None, emissive=(0, 0, 0), emissive_strength=1.0,
#   glow_light=1.0, glow_shadows=True
# and draw_distance=0.0 (metres; 0 = drawn at any distance)
# and material=None: a shared .mat.ron file that sets colour, surface and
#   textures (cube(material="materials/brick.mat.ron"))
m.kind                      # "cube", "sphere", "quad" or "model"
m.color                     # (r, g, b, a); tints a model
m.roughness                 # 0 = mirror .. 1 = chalk, or None (default)
m.metallic                  # 0 = plastic/wood/stone, 1 = metal, or None
m.emissive                  # (r, g, b) glow colour, (0, 0, 0) = none
m.emissive_strength         # glow multiplier
m.glow_light                # how much the glow lights its surroundings
m.glow_shadows              # whether objects block that light (HIGH and up)
m.draw_distance             # hidden past this camera distance, 0 = never
m.material                  # the material file's path, or None (own colour)
m.path                      # the model file, or None
m.bounds                    # ((min x, y, z), (max x, y, z)) or None

at.set_asset_root(path)     # where relative model paths resolve (None resets)
at.asset_root()             # the current root, or None
```

Plain data; materialized on the GPU by the windowed driver, inert headless.
`Transform.scale` sizes the primitive.

Surface arguments: `None` roughness / metallic means "default", which is
0.6 / 0 for primitives and the file's own material values for models.
Setting them overrides every part of a model. Roughness and metallic must be
in `0..=1`, and emissive values must not be negative, or `ValueError` is
raised. A glowing mesh also lights the surfaces around it like a real
glowing object of its size; `glow_light` scales that (0 = off). On HIGH and
up, objects in the way block that light and cast shadows from it, unless
`glow_shadows=False`.
`draw_distance` hides the mesh, shadow included, when the camera is farther
away than that many metres; it dithers out over the last tenth instead of
popping. It must not be negative. Model files can also carry simpler levels
of detail ([Models](oberon/models.md#levels-of-detail)).
The glow added is `emissive × emissive_strength`; above 1 it blooms
on Ultra and Beautiful. Guide: [Rendering & Lights](guides/rendering-and-lights.md#surfaces).

`material=` names a [material file](oberon/materials.md) (relative to the
asset root, like model paths). It replaces the colour and surface arguments
(and a model's own materials) and is read right away: a missing file raises
`FileNotFoundError`; a broken one, or a path not ending in `.mat.ron`,
raises `ValueError`.

`model` loads the file right away, so a missing file raises
`FileNotFoundError` and a broken one raises `ValueError` where you create the
component. Relative paths resolve against the asset root.
`Scene.load` / `run_scene` set the root to the scene's folder. If no root is
set, the folder of the running script is used, then the working directory.
The file is cached, so a thousand trees load it once. Guide:
[Models](oberon/models.md).

### DirectionalLight

```python
at.DirectionalLight(direction=(-0.5, -1.0, -0.35),
                    color=(1.0, 1.0, 1.0), ambient=0.3, shadows=True,
                    *, shadow_distance=120.0, shadow_softness=1.0)
```

The scene's sun; the first entity carrying one lights the frame. Getters:
`direction`, `color`, `ambient`, `shadows`, `shadow_distance`,
`shadow_softness`. Without one, a neutral shadowless default is used.
`shadow_distance` (metres, above 0) is how far shadows reach; shorter is
sharper. `shadow_softness` (0..8, default 1) is how soft their edges are.
Out-of-range values raise `ValueError`. Guide:
[Rendering & Lights](guides/rendering-and-lights.md#the-sun).

### PointLight / SpotLight

```python
at.PointLight(color=(1.0, 0.85, 0.65), intensity=3.0, range=0.0,
              *, radius=0.05, shadows=True)
at.SpotLight(color=(1.0, 1.0, 1.0), intensity=8.0, outer_angle=30.0,
             inner_angle=None, range=0.0, *, radius=0.05, shadows=True)
```

Placed lights. A `PointLight` shines every way from its entity's position.
A `SpotLight` shines along its entity's forward axis (local −Z, turned by
the entity's rotation). `intensity` 1 lights a surface 1 m away like a
white sun overhead, falling off with the square of the distance. `range`
(metres) ends the light; 0 works it out from the brightness.
`outer_angle` is the cone's edge in degrees from its axis (0–80), and
`inner_angle` is the full-strength core (default ¾ of `outer_angle`).
`radius` is the size of the glowing source. `shadows` lets objects block
the light (High and up, nearest first). Getters have the same names.
Negative values, a zero radius, or angles out of range raise `ValueError`.
Saved as `point_light` / `spot_light` in scene files. Guide:
[Rendering & Lights](guides/rendering-and-lights.md#point-lights-and-spotlights).

### Environment

```python
at.Environment(sky_color=(1, 1, 1), ground_color=(1, 1, 1),
               background=(0.012, 0.012, 0.012), exposure=1.0,
               tonemapping="none", fog=False, fog_color=(0.5, 0.55, 0.6),
               fog_density=0.02, fog_start=0.0, sky=False,
               zenith_color=(0.16, 0.34, 0.78), sun_size=2.0, sun_glow=1.0,
               clouds=0.0, lens_flare=1.0, ambient_occlusion=1.0,
               fog_height=0.0, fog_falloff=0.0, fog_sun=0.0, god_rays=0.0,
               depth_of_field=0.0, focus_distance=0.0, focus_range=1.0,
               toon_bands=0, toon_softness=0.15, shadow_tint=(1, 1, 1),
               rim_light=0.0, painted_sky=False, outline=0.0,
               outline_color=(0.05, 0.04, 0.06), pixelate=0.0, palette=0,
               post=None, bloom=None, contrast=None, saturation=None,
               temperature=None, tint=None, lift=None, gamma=None, gain=None,
               vignette=None, grain=None, chromatic_aberration=None)
at.Environment.preset("neutral" | "daylight" | "dusk" | "night")
env.with_post(post=None, *, bloom=None, ...)   # same post keywords
at.Environment.post_presets()  # ["neutral", "bleached", "vivid", "film", "noir"]
at.Environment.style(name, base=None)   # "realistic", "semi-realistic", "botw", "a short hike", "nier"
env.with_style(name)
at.Environment.styles()        # the five names, in the editor's order
env.style_name                 # the style it is exactly, or "Custom"
```

The scene look. The first entity carrying one wins. It sets hemisphere
ambient (sky above, ground below, scaled by the sun's `ambient`), the
background clear colour (overriding `Game.clear_color`), exposure, tone
mapping (`"none"`, `"reinhard"`, `"aces"`, `"agx"`), and exponential
distance fog. `sky=True` draws a sky dome instead of the flat background:
a gradient from `background` (the horizon) to `zenith_color`, a sun disk
`sun_size` degrees wide where the scene's `DirectionalLight` shines from,
a halo (`sun_glow`), `clouds` cover from 0 to 1, and on `BEAUTIFUL` a lens
flare (`lens_flare` strength) when the camera looks toward the sun. Also
on `BEAUTIFUL`, `ambient_occlusion` (0 off, 1 default, up to 2) darkens
the ambient light in creases, corners and on the ground around objects.
The non-neutral presets turn the sky on.

Height fog: with `fog_falloff` above 0 the fog has full density below
`fog_height` and thins by e every `1 / fog_falloff` metres above it, so it
lies in valleys. `fog_sun` (0..4) makes it glow toward the sun. On
`BEAUTIFUL`, `god_rays` (0..2) draws sun shafts blocked by geometry, and
`depth_of_field` (0..4) blurs what's nearer or farther than
`focus_distance` ± `focus_range` metres (`focus_distance=0` follows the
camera's target). The `"daylight"` and `"dusk"` presets turn on height fog
and god rays. See
[Atmosphere](guides/rendering-and-lights.md#atmosphere-height-fog-god-rays-depth-of-field-dust).

Post effects grade the finished frame on `MEDIUM` and up: `post` starts
from a named look, and the other post keywords change single fields on
top of it (unset ones keep the look's value, or the neutral default
without one). Fields are `bloom` (0..4, `HIGH` and up), `contrast`,
`saturation` (0..3), `temperature` and `tint` (-1..1), `lift`, `gamma` and
`gain` (RGB tuples), and `vignette`, `grain` and `chromatic_aberration`
(0..1). `with_post` returns a copy with the post effects changed and
everything else kept.

Styles: `Environment.style(name, base=None)` applies a whole look to
`base` (the default environment if omitted). It sets tone mapping,
exposure, the lens, the height fog's shape, shading, outlines, pixels
and post effects, and keeps the scene's colours, fog colour and density,
sky and clouds. `with_style` does the same to a copy, and `style_name`
reads back which style an environment is, or `"Custom"`. The style's own
fields: `toon_bands` (0..4 flat light steps, 0 = smooth) with
`toon_softness` (0..1); `shadow_tint`, the colour of shade (RGB, white =
none); `rim_light` (0..2); `painted_sky` (bool); and three screen effects
for `MEDIUM` and up: `outline` (pixels on a 720 px screen, 0 = none) in
`outline_color`, `pixelate` (pixel size on a 720 px screen, 0 = off) and
`palette` (dithered levels per channel, 0 = full colour). See
[Styles](guides/rendering-and-lights.md#styles-from-realistic-to-toon-and-pixel-art).

All arguments are readable as properties. Unknown tone mapping, preset,
look or style names raise `ValueError`. It's saved in scene files.
Guide: [Rendering & Lights](guides/rendering-and-lights.md#environment).

### Water

```python
at.Water(size=(20, 20), deep_color=(0.01, 0.07, 0.11),
         shallow_color=(0.1, 0.42, 0.44), clarity=2.5, wave_height=0.12,
         wave_length=5.0, wave_speed=1.0, wave_direction=30.0, foam=0.6,
         density=2.0, drag=1.0, endless=False, caustics=1.0, wind=0.0)
at.Water.preset("pond" | "lake" | "ocean" | "sea" | "default")
```

A rectangle of animated water on the entity's local XZ plane, `size`
metres wide, centred on its `Transform`. Waves are Gerstner waves
(`wave_height` crest to rest, `wave_length` of the main wave, travelling
toward `wave_direction` degrees around +Y). Every quality preset draws it;
`BEAUTIFUL` adds refraction of what's underneath, depth-based absorption
(`clarity` is how many metres you can see down) and shoreline `foam`.
`endless=True` (on the ocean and sea presets) continues the surface,
flat and calm, out to the horizon; waves, foam and physics stay within
`size`. `caustics` (0 to 2) is the strength of the light the waves focus on
what's under them (Medium and up). `wind` (0 to 1) blows spray off the
crests from 0.5 up (the Environment's `wind` too, whichever is stronger). With the camera under the surface the view fogs
blue-green and the sound is muffled. It's also a volume of water for physics. Dynamic bodies in it float by
buoyancy (`density` against their colliders' density, which defaults to
1.0, so a default box floats half under) and are slowed by `drag`.
All arguments are readable as properties, and it's saved in scene files.
Guides: [Rendering & Lights](guides/rendering-and-lights.md#water),
[Physics](guides/physics.md#water).

### Terrain

```python
at.Terrain(size=64.0, resolution=129, layers=None, roughness=0.9, collide=True)
t.shape(lambda x, z: height) -> Terrain       # every height from a function
t.sculpt(tool, (x, z), radius=4.0, amount=1.0, height=None) -> Terrain
t.paint(layer, (x, z), radius=4.0, strength=1.0) -> Terrain
t.auto_paint(water_level=0.0, beach=1.5, cliff=38.0) -> Terrain
t.load_heightmap(path, low=0.0, high=10.0) -> Terrain
t.with_resolution(resolution) -> Terrain     # resampled, same shape
t.height_at(x, z) -> float
t.normal_at(x, z) -> (x, y, z)
t.weights_at(x, z) -> (grass, rock, sand, dirt)
t.size, t.resolution, t.spacing, t.heights, t.height_range, t.layers,
t.roughness, t.collide

at.TerrainLayer(color=(1, 1, 1), texture="", tile=4.0)
```

Heightmap ground over a square `size` metres a side, centred on the
entity, with `resolution` heights a side (3 to 1025). Positions are
`(x, z)` in the entity's space. `sculpt` tools: `"raise"` and `"lower"`
(by `amount` metres at the centre), `"smooth"` (`amount` 0..1),
`"flatten"` (toward `height`, by default the ground's height at the
centre) and `"noise"`. `layer` is 0..3 or `"grass"`, `"rock"`, `"sand"`,
`"dirt"`. `auto_paint` puts sand up to `beach` metres above
`water_level`, rock on slopes steeper than `cliff` degrees, dirt just
below that and grass elsewhere. Every method returns a changed copy.
With `collide` it's a fixed heightfield collider: bodies, characters and
ray casts meet it. `Grass` and `Scatter` over it stand on it. Saved in
scene files as two base64 PNGs.
Guide: [Terrain](oberon/terrain.md#from-python).

### Grass

```python
at.Grass(area=(0, 0), density=60.0, height=0.6, height_variation=0.45,
         width=0.07, base_color=(...), tip_color=(...), color_variation=0.3,
         draw_distance=60.0, seed=0, follow_terrain=True)
grass.paint(position, radius=2.0, normal=(0, 1, 0), strength=1.0) -> Grass
grass.erase(position, radius=2.0, strength=1.0) -> Grass
grass.blade_count() -> int
grass.strokes  # how many brush strokes
```

A field of blades over `area` (metres along the entity's X and Z,
centred on it) plus brush strokes in the entity's space. `paint` and
`erase` return changed copies. The blades sway in the Environment's
`wind`, bend away from characters and moving bodies, and thin out toward
`draw_distance`. With `follow_terrain` (the default) blades stand on a
`Terrain` under them. Saved in scene files with its strokes.
Guide: [Grass & Foliage](guides/grass-and-foliage.md).

### Scatter

```python
at.Scatter(area=(0, 0), density=0.2, scale=(0.7, 1.3), align=0.3, seed=0,
           follow_terrain=True)
scatter.paint(...) / scatter.erase(...)  # as Grass
scatter.count() -> int
```

Draws the entity's `MeshRenderer` at many spots on the ground instead of
once (rocks, flowers, trees), each copy turned and sized at random
between `scale`'s ends and tilted toward the slope by `align`. Copies
aren't solid. Guide: [Grass & Foliage](guides/grass-and-foliage.md#scatter).

### ParticleEmitter

```python
at.ParticleEmitter(**fields)                 # any field below by keyword
at.ParticleEmitter.preset("fire" | "smoke" | "sparks" | "dust" | "splash" | "magic" | "motes", **fields)
at.ParticleEmitter.presets() -> list[str]
emitter.replace(**fields) -> ParticleEmitter # a changed copy
emitter.preset_name                          # "fire" … or None once changed
```

Fields (all readable as properties): `emitting`, `rate`, `burst`,
`duration`, `one_shot`, `max_particles`, `lifetime` (min, max),
`shape` (`"point"`, `"sphere"`, `"box"`, `"cone"`), `shape_size`,
`cone_angle`, `speed` (min, max), `gravity`, `drag`, `size` (birth, death),
`size_spread`, `color`, `color_end` (RGBA, over 1 glows), `fade_in`,
`fade_out`, `spin`, `stretch`, `blend` (`"alpha"`, `"additive"`), `lit`,
`soft`, `sprite` (`"dot"`, `"spark"`, `"star"`, `"flame"`, `"puff"`),
`texture`, `sheet` (columns, rows), `world_space`, `follow_camera` (fill a box of `shape_size` around the camera, wrapping particles round as it moves). Pairs also take one number for both, and colours take RGB or RGBA. A bad name raises `KeyError`. A bad value raises `ValueError`. Saved in scene
files. Guide: [Rendering & Lights](guides/rendering-and-lights.md#particles).

### Animation

```python
at.Animation(clip="", speed=1.0, looping=True, play_on_start=True, root_motion=False,
             graph="", foot_ik=False)
```

Plays a clip of the entity's animated model (a glTF with a skeleton or
animated nodes on its `MeshRenderer.model`). `clip` is the clip played on
start; `""` shows the rest pose. `speed` is the playback rate (negative plays
backwards). A clip with `looping=False` plays once and holds its last frame.
`root_motion=True` lets the clip's travel across the ground move the entity
(through its Character Controller if it has one) instead of sliding the
model away from it. All five are readable properties. Saved in scene files. While the game runs,
`view.animation` controls it (below). Guide:
[Models → Animation](oberon/models.md#animation).

Spring bones (hair, skirts and tails that swing on their own) aren't a
component: they're set up in the model's import settings
(`<model>.import.ron`), so every entity showing the model gets them.
`view.animation.springs` says whether it has any, and
`view.animation.reset_springs()` puts them back at rest. Guide:
[Models → Spring bones](oberon/models.md#spring-bones).

Clips can also come from other glTF files made for the same skeleton,
listed under `clips` in the model's import settings. A file with one clip
names it after the file (`anims/Run Fast.glb` gives `"Run Fast"`); a file with
several gives `"file:Clip"` (`"sword:Slash"`). They're in `anim.clips` and play
like the model's own. Guide:
[Models → Clips from other files](oberon/models.md#clips-from-other-files).

Animation events are named moments in clips, set under `events` in the
import settings (`(clip: "Run", time: 0.18, name: "footstep")`). A clip
played once also fires `"end"`. `view.animation.recent_events` lists the
latest as `(seq, clip, name)`, with `seq` counting up per entity. Behaviour
scripts get them as `on_animation_event` calls and channel messages
(below). Guide: [Models → Animation events](oberon/models.md#animation-events).

`foot_ik=True` plants each foot on what's under it (slopes, stairs). It
stands on colliders while the game runs, and on meshes in a game without
physics.

```python
at.Attach(to, bone="")
```

Makes the entity follow `bone` of the entity named `to` (no bone: that
entity's origin). Its `Transform` becomes its offset in the bone's space,
and it keeps its own size. `to` and `bone` are readable. Saved in scene
files. Guide: [Attachments & IK](oberon/attachments-and-ik.md).

### Navigation

```text
at.Navigation(file="", agent_radius=0.35, agent_height=1.8, max_climb=0.4,
              max_slope=45.0, jump_distance=2.5, jump_height=1.2,
              drop_height=3.0, cell_size=0.2, min_area=0.5)
at.NavAgent(speed=3.5, acceleration=12.0, radius=0.4, hover=0.0,
            stop_distance=0.1, avoid=True, turn=True)
at.NavLink(to, two_way=True)
```

`Navigation` (one per scene) says how the navigation mesh is baked from
the static colliders and, in `file`, where it was saved (relative to the
scene; empty: baked when the game starts). `NavAgent` makes an entity walk
the mesh by itself: a character through its controller, anything else
`hover` metres above the mesh. `NavLink` is a hand-placed jump from this
entity to the one named `to`. All are saved in scene files and readable.
Guide: [Navigation](oberon/navigation.md).

```text
ctx.physics.nav_ready -> bool
ctx.physics.nav_path(start, end, partial=False) -> [(x, y, z, is_link)] | None
ctx.physics.nav_nearest(point, max_distance=5.0) -> (x, y, z) | None
view.nav_state           # dict | None: the agent's speed, target, path, ...
view.nav_edit(go=..., speed=..., stop_distance=..., hover=...)
```

Scripts use the friendlier `entity.nav` and `ctx.nav` (below).

### Ragdoll

```text
at.Ragdoll(active=False, mass=70.0, stiffness=0.0, blend_time=0.5)
r.active, r.mass, r.stiffness, r.blend_time
```

Lets an entity with an animated model go limp: capsules and limited
joints are built from its skeleton, and physics moves every bone. `mass`
(kg, 1 to 10000) is shared out by the capsules' sizes. `stiffness` (0 to 1)
is how hard the joints hold the pose it went limp in. `blend_time`
(seconds, 0 to 10) is how long the animation takes to take over again.
`active=True` starts it limp. Saved in scene files.
Guide: [Ragdolls](oberon/ragdolls.md).

```text
view.ragdoll_state        # dict | None: on, mass, stiff, blend, limp, w, rest, hips
view.ragdoll_edit(on=..., mass=..., stiffness=..., blend_time=...,
                  push=((x, y, z), bone_or_None))
```

Scripts use the friendlier `entity.ragdoll`: `.on(push=None, bone=None)`,
`.off()`, `.push(impulse, bone=None)`, `.limp`, `.resting`, `.blending`,
`.weight`, `.hips`, and settable `.active`, `.mass`, `.stiffness`,
`.blend_time`.

### Physics components

```text
at.RigidBody(body_type="dynamic", *, gravity_scale=1.0,
             linear_damping=0.0, angular_damping=0.0, enabled=True)
at.RigidBody.dynamic() / .fixed() / .kinematic_velocity() / .kinematic_position()
body.body_type

at.Collider3D.cuboid(x, y, z)      # FULL sizes
at.Collider3D.ball(radius)
at.Collider3D.capsule(half_height, radius)
at.Collider3D.mesh(model)          # a model's triangles (static scenery)
at.Collider3D.convex_hull(model)   # a model's convex hull (moving bodies)
# modifiers chain: .sensor(bool) .density(d) .friction(f) .restitution(r)
#                  .offset(x, y, z)   shape centre relative to the entity

at.Collider2D.cuboid(x, y) / .ball(radius) / .capsule(half_height, radius)
# same modifiers (.offset(x, y))

at.CharacterController(mode="normal", max_slope_climb_angle=0.7853982,
                       min_slope_slide_angle=0.5235988, autostep=None,
                       snap_to_ground=None, offset=0.05, slide=True)

at.CharacterVelocity(velocity=(0, 0, 0))   # .velocity get/set

at.Joint(kind="hinge", *, connected=None, anchor=(0, 0, 0),
         connected_anchor=None, axis=(0, 1, 0), limits=None, length=0.0,
         stiffness=30.0, damping=2.0, motor=None, collide_connected=False,
         break_force=0.0, enabled=True)
at.Joint.hinge(axis=(0, 1, 0), **opts) / .ball(**opts) / .slider(axis=(1, 0, 0), **opts)
at.Joint.fixed(**opts) / .rope(length=0.0, **opts)
at.Joint.spring(length=0.0, stiffness=30.0, damping=2.0, **opts)
joint.replace(**opts)              # a copy with some options changed
# connected: an entity name, None = the world. anchor: local to this entity.
# limits: radians (hinge, ball) or metres (slider). length 0 = starting distance.
# break_force: newtons, 0 = never. An unknown option raises TypeError.

at.JointMotor(speed=0.0, *, target=None, stiffness=400.0, damping=80.0,
              max_force=0.0)       # target set: pull to it; else turn at speed

view.joint -> JointRef | None      # kind, component, value, angle, attached,
                                   # broken, force (N); settable: connected,
                                   # enabled, motor_speed, motor_target,
                                   # limits, break_force
```

## FrameContext (`ctx`)

Passed to every update callback. Reads are start-of-frame snapshots
(reading an entity you just wrote returns your pending write); writes apply
after the callback.

| Member | Contents |
|---|---|
| `ctx.time` | `FrameTime`: `delta` (s), `elapsed` (s), `frame` (1-based) |
| `ctx.input` | `Input` snapshot (below) |
| `ctx.scene` | `SceneView` (below) |
| `ctx.events` | `Events`: `send(name, payload=None)`, `read(name) -> list` |
| `ctx.resources` | `Resources`: `set/get/remove/clear/names`, `in`, `len` |
| `ctx.physics` | 3D physics queries (below) |
| `ctx.physics2d` | 2D physics queries |
| `ctx.audio` | `Audio`: play and control sounds (below) |
| `ctx.quit()` | ends the loop |

### SceneView / EntityView

```text
ctx.scene.entity_count
ctx.scene.names() -> list[str]
ctx.scene.find(name) -> EntityView | None
ctx.scene["name"]                    # KeyError when missing
for view in ctx.scene: ...
ctx.scene.spawn(entity)              # appears next frame
ctx.scene.despawn(id)
ctx.scene.load(scene)                # switch levels; the next frame sees only `scene`
ctx.scene.burst(effect, position, direction=None, count=None)
                                     # a one-off particle effect (a preset name or
                                     # ParticleEmitter) that cleans up after itself
ctx.scene.burst_emitter(id, count)   # extra particles from an entity's own emitter
```

```text
view.id                  # stable while alive
view.name
view.transform           # TransformRef; assign a Transform to replace
view.transform.position / .rotation_euler / .scale    # get/set
view.transform.set(transform); view.transform.copy()
view.character_velocity  # (vx, vy, vz) | None; assign to drive characters
view.camera_3d           # Camera3D copy | None; assign to move the camera
view.animation           # AnimationRef | None (entities with an Animation and an animated model)
view.attach(to, bone="", offset=None)   # follow a bone from next frame (offset: Transform)
view.detach()            # stop following, staying put
view.attached_to         # (name, bone) | None
view.despawn()           # queues removal (children too)
```

```text
anim.play(clip, loop=True, fade=0.0, speed=None, restart=False)
                         # blend to `clip` over `fade` seconds; KeyError for an unknown clip.
                         # Asking for the clip that's playing keeps it going (unless restart)
anim.stop(fade=0.0)      # blend back to the rest pose
anim.clip                # the clip playing (or held), or None
anim.time                # seconds into the clip; assign to jump
anim.duration            # the clip's length; anim.length("Jump") for any clip
anim.playing             # False once a one-shot ends or after stop()
anim.speed               # get/set; anim.looping
anim.clips               # every clip in the model, in file order, then borrowed ones
anim.look_at(target, weight=1.0)        # turn the head: a name, an EntityView or (x, y, z);
                                        # None eases back
anim.reach(bone, target, weight=1.0)    # bend the limb ending at `bone` to reach target
anim.foot_ik             # get/set: feet on the ground
anim.looking; anim.reaching             # whether looking; the bones reaching
```

Reads show the start of the frame. `play`, `stop` and setting `time` or
`speed` apply after your update.

### Input

```python
ctx.input.pressed(key) / just_pressed(key) / just_released(key)
ctx.input.mouse_pressed(btn) / mouse_just_pressed(btn) / mouse_just_released(btn)
ctx.input.cursor        # (x, y) | None
ctx.input.scroll        # (dx, dy)
ctx.input.modifiers     # .shift .ctrl .alt .super_key
ctx.input.gamepads      # Gamepads (below)
ctx.input.touches       # [Touch]: fingers, oldest first (below)
```

`Touch`: `finger` (0 for the first; kept while down), `position`, `start`
(pixels), `moved`, `just_started`, `just_ended`, `primary` (the finger
driving the left mouse button). Without a touch screen the held left mouse
button is finger 0.

`Gamepads`: `ids()`, `get(id)`, `primary()`, `any_connected()`,
`pressed(id, button)`, `pressed_any(button)`, `axis(id, axis)`,
`axis_any(axis)`, `events`, `len`, iteration.
`Gamepad`: `id`, `name`, `pressed(button)`, `axis(axis)`,
`left_stick()`, `right_stick()`.
`GamepadEvent`: `kind` ("connected"/"disconnected"/"button"/"axis"),
`id`, `name`, `button`, `pressed`, `axis`, `value`.

### Physics (ctx.physics)

```text
ctx.physics.grounded(entity) -> bool
ctx.physics.velocity(entity) -> Vec3
ctx.physics.set_velocity(entity, (vx, vy, vz))
ctx.physics.apply_impulse(entity, (ix, iy, iz))
ctx.physics.apply_force(entity, (fx, fy, fz))
ctx.physics.gravity() -> Vec3
ctx.physics.set_gravity((gx, gy, gz))
ctx.physics.body_count() -> int
key = ctx.physics.cast_ray(origin, direction, max_distance=1000.0, exclude=None)
ctx.physics.ray_result(key) -> (entity_id, point, normal, distance) | None
ctx.physics.water_height(x, z) -> float | None   # surface height, waves included
ctx.physics.splash((x, y, z), strength=1.0)      # rings and foam, from the next step
```

`ctx.physics2d`: `apply_impulse(entity, (x, y))`, `grounded(entity)`,
`body_count()`. Entities are `EntityView`s or integer ids.

### Audio (ctx.audio)

Plays and controls sounds. Unlike scene/physics writes, audio operations apply
**immediately** and queries observe live state — including within the frame
that issued them. `play` returns an integer sound id used by the other
methods. Volume is a linear amplitude (`1.0` = full, `0.0` = silent).

```python
sid = ctx.audio.play("assets/jump.wav")          # -> sound id (int)
ctx.audio.play("assets/music.ogg", volume=0.6, loop_=True)
ctx.audio.play("assets/beep.wav", playback_rate=1.5, panning=-0.3)

ctx.audio.stop(sid, fade_out=0.2)                # fade out over 0.2 s, then stop
ctx.audio.stop_all(fade_out=0.0)                 # stop every sound
ctx.audio.pause(sid) / ctx.audio.resume(sid)
ctx.audio.seek_to(sid, 1.5)                      # seconds
ctx.audio.set_volume(sid, 0.8, fade=0.1)
ctx.audio.set_playback_rate(sid, 0.9)
ctx.audio.set_panning(sid, 0.5)                  # -1.0 left .. 1.0 right
ctx.audio.set_master_volume(0.7, fade=0.5)
ctx.audio.set_low_pass(800.0, fade=0.2)          # muffle everything (Hz)
ctx.audio.set_low_pass(None, fade=0.2)           # ... and clear it again

ctx.audio.is_playing(sid) -> bool
ctx.audio.position(sid) -> float | None          # seconds; None if unknown
ctx.audio.sound_count() -> int
ctx.audio.low_pass -> float | None               # the cutoff, or None if clear
ctx.audio.underwater -> bool                     # the camera's under water
```

Supported formats: WAV, OGG/Vorbis, MP3, FLAC, PCM. A missing or undecodable
file raises `ValueError`; an unknown sound id raises `KeyError`.

**Backend:** the default build runs a *silent* backend — no audio device is
touched, no sound is produced, but playback positions and states still advance
deterministically, so headless runs and CI exercise identical code paths. Real
device output requires the `audio-cpal` build feature (`scripts/build.py`
enables it automatically when ALSA headers are present). If a device build
finds no output device, it falls back to silent rather than failing.

## Input mapping

```text
controls = at.ActionMap()
controls.bind(action, binding)        # stacks
controls.bind_axis(name, axis)        # stacks; values sum, clamp -1..1
controls.rebind(action, bindings)     # replace
controls.unbind(action)  /  controls.unbind_axis(name)
controls.bindings(action)  /  controls.actions()  /  controls.axis_names()
controls.pressed(action, ctx.input)
controls.just_pressed(action, ctx.input)
controls.axis(name, ctx.input) -> float
controls.axis_pair(x, y, ctx.input) -> (float, float)
action in controls  /  len(controls)
```

```text
at.Binding.key(key) / .mouse(button) / .gamepad_button(button)
at.Binding.gamepad_axis_positive(axis) / .gamepad_axis_negative(axis)
at.AxisBinding.key_pair(neg_key, pos_key)
at.AxisBinding.button_pair(neg_binding, pos_binding)
at.AxisBinding.gamepad_axis(axis, scale=1.0)
```

Unknown key/button/axis names raise `KeyError` at bind time.

## Enums

All are `str` enums — members work anywhere the plain string does.

| Enum | Members (sample) | Full set |
|---|---|---|
| `at.Key` | `A`–`Z`, `DIGIT_0`–`9`, `SPACE`, `ENTER`, `ESCAPE`, `TAB`, arrows, `F1`–`F12`, modifiers, punctuation | 80 |
| `at.MouseButton` | `LEFT`, `RIGHT`, `MIDDLE`, `BACK`, `FORWARD` | 5 |
| `at.GamepadButton` | `SOUTH`, `EAST`, `NORTH`, `WEST`, triggers, thumbs, d-pad, `START`, `SELECT`… | 19 |
| `at.GamepadAxis` | `LEFT_STICK_X/Y`, `RIGHT_STICK_X/Y`, `LEFT_Z`, `RIGHT_Z`, `DPAD_X/Y` | 8 |
| `at.GraphicsQuality` | `LOW` (no shadows), `MEDIUM` (1 × 1024), `HIGH` (2 cascades × 2048), `ULTRA` (3 cascades × 2048), `BEAUTIFUL` (contact-hardening shadows, ambient occlusion, PBR, bloom, refractive water) | 5 |

## Behaviour scripts (`automata.run_scene`)

```python
at.run_scene(path, *, max_frames=None, update=None, **game_options) -> int
```

Plays an Oberon scene file with its behaviour scripts:
`start(entity, ctx)`, `update(entity, ctx)`, `stop(entity, ctx)`, and
`on_animation_event(entity, ctx, name)` for the entity's own animation
events, and `on_joint_broken(entity, ctx)` when its joint snaps. It returns
the number of frames run.

| `entity` | |
|---|---|
| `name`, `id`, `state` (dict), `grounded`, `script` | |
| `transform` | `position`, `rotation_euler` (radians), `scale`; `translate(d)`, `rotate(d)`. Read, or assign |
| `character_velocity` | Set to move a character |
| `attributes` | Dict-like, saved in the scene |
| `camera` | `ScriptCamera` or `None` |
| `audio` | `play()`, `stop()`, `playing`, `source` |
| `animation` | `play(clip, loop=True, fade=0.0, speed=None, restart=False)`, `stop(fade=0.0)`, `clip`, `time` (settable), `duration`, `length(clip)`, `playing`, `speed` (settable), `looping`, `clips`, `springs`, `reset_springs()`, `root_motion` (settable), or `None` without an Animation. Reads include this frame's requests |
| `nav` | `go(target)` (a point, an entity or a name; `None` stops), `stop()`, `target`, `arrived`, `moving`, `jumping`, `velocity`, `path`, `remaining_distance`, and settable `speed`, `stop_distance`, `hover`; or `None` without a Nav Agent ([Navigation](oberon/navigation.md#nav-agents)) |
| `joint` | `kind`, `connected` (settable), `value`, `angle`, `attached`, `broken`, `force`, and settable `enabled`, `motor_speed`, `motor_target`, `limits`, `break_force`; or `None` without a Joint ([Physics → Joints](guides/physics.md#joints)) |
| `camera_3d` | The camera as a `Camera3D` (get/set) |
| `view` | The engine's `EntityView` for this frame |
| `despawn()` | At the end of the frame |

| `ctx` | |
|---|---|
| `time` | `delta`, `elapsed`, `frame` (from 1) |
| `scene` | Script entities: `["Name"]`, `find`, `by_id`, `in`, iteration |
| `input` | Keys with the aliases `"shift"`, `"ctrl"`, `"alt"` (either side) and `"left"`/`"right"`/`"up"`/`"down"`, or actions bound with `settings.bind`; `axis(neg, pos)`; `any_just_pressed()` (the new key's name, for rebinding); mouse buttons, `mouse_delta`, `lock_mouse`; `touches` (fingers, or the left mouse button as finger 0); `touch_controls` (on-screen sticks and buttons: `set(sticks, buttons, look, show)`, `visible`, `stick(side)`, `configured`; see [Touch controls](guides/touch-controls.md)) |
| `physics` | `grounded`, `raycast`, `water_height(x, z)`; the rest of the engine's physics, which also accepts script entities |
| `nav` | `ready`, `path(start, end, partial=False)` (corner points, or `None`), `path_length(start, end)`, `nearest(point, max_distance=5.0)`, `reachable(start, end)`; points may be entities ([Navigation](oberon/navigation.md#paths-from-scripts)) |
| `ai` | State machines (`state_machine(states, initial)`), behaviour-tree nodes (`sequence`, `selector`, `parallel`, `action`, `condition`, `wait`, `invert`, `succeed`, `repeat`, `cooldown`; `SUCCESS`, `FAILURE`, `RUNNING`), sight (`can_see(viewer, target, fov=120, distance=20, eye=1.6, aim=1.0, forward=(0, 0, -1), walls=True)`, `visible(viewer, targets=None, ...)`) and hearing (`noise(position_or_entity, radius=10, kind="noise")`, `hear(listener, hearing=1.0, kinds=None)`, `noises`). See [AI helpers](#ai-helpers-ctxai-automataai) |
| `water` | `height(x, z)` (same as `physics.water_height`), `splash(position_or_entity, strength=1.0)`: rings and foam at that point (strength 0–10; 1 is about a crate dropped from a couple of metres) |
| `particles` | `burst(effect, position_or_entity, direction=None, count=None, **fields)`: a one-off effect (a preset name or `at.ParticleEmitter`, with fields changed) that cleans up after itself; `emit(entity, count)`: extra particles from the entity's own emitter |
| `audio`, `hud` | One-shot sounds; HUD images (`"builtin:circle"` and `"builtin:ring"` are built in), rectangles and text (`hud.text(text, x, y, size=24, color, align, shadow, outline)`) |
| `camera` | The first `Camera3D` in the scene as a `ScriptCamera` |
| `events`, `resources`, `screen`, `raw`, `quit()` | Engine extras (`run_scene` only). `screen.measure_text(text, size)` gives text's pixel size |
| `subscribe(pattern, fn)` | Calls `fn(msg)` for each message on a matching channel: an exact name, or `*` and `?` wildcards. Animation events arrive as `"<entity>:animation:<event>"`. `msg` has `channel`, `data`, `ctx`, and for animation events `entity`, `event` and `clip`. Subscribing the same function again does nothing; the subscription ends with the subscribing entity. Returns a `Subscription` (`cancel()`) |
| `unsubscribe(pattern, fn=None)` | Ends this script's subscriptions to `pattern` (or the `Subscription` passed); returns how many |
| `publish(channel, data=None)` | Delivers to every subscriber now; returns how many got it |
| `tween(entity, field, to, duration, ease="out_cubic", *, start=None, delay=0, loop=False, yoyo=False, on_update=None, on_done=None)` | Moves `"position"`, `"rotation"`, `"scale"`, an attribute, or a key's `component:field` to `to` along an easing curve. `loop=True` repeats forever, `loop=n` plays n times; `yoyo` runs every other play backwards. A new tween on the same entity and field replaces the old. Returns a `Tween`: `value`, `progress`, `done`, `cancel()`, `finish()`, `pause()`, `resume()` |
| `tween_value(start, end, duration, on_update=None, ease="out_cubic", *, delay, loop, yoyo, on_done)` | A tween of your own value: read `.value` |
| `after(seconds, fn, *args)`, `every(seconds, fn, *args, times=None)` | Calls `fn(*args)` once later, or every `seconds`. Returns a `Timer`: `remaining`, `count`, `done`, `cancel()`, `pause()`, `resume()`. Timers and tweens move before `update`, and end with the entity whose script made them |
| `save` | A dict kept between runs as JSON (`save["coins"] = 12`): numbers, strings, booleans, `None`, lists and dicts. Written at the end of the frame if changed. Slots: `use(slot)`, `slot`, `slots()` (`SaveSlot`: `name`, `saved_at`, `keys`), `exists(slot)`, `delete(slot)`, `clear()`, `flush()`, `saved_at`, `path`. Lives in `.oberon/save` while developing and in the user-data folder once exported (`automata.script_api.user_data_dir(game)`; `$AUTOMATA_SAVE_DIR` overrides it). `run_scene(save_dir=...)` picks the folder |
| `settings` | The player's settings, saved and applied at start: `volume` (0–1, master volume), `graphics` (preset name), `[name]` for your own, `reset()`. Key bindings: `bind(action, *keys)` (defaults), `rebind(action, *keys)` (saved), `keys(action)`, `bindings()`, `reset_keys(action=None)` |
| `ease(name, t)` | `t` through a curve: `"linear"`, or `in_`/`out_`/`in_out_` + `sine`, `quad`, `cubic`, `quart`, `quint`, `expo`, `circ`, `back`, `elastic`, `bounce` (`automata.script_api.EASINGS`) |
| `get(key)`, `set(key, value)` | One exact key, `entity:component:field`. Components: `animation` (`clip` (setting it plays), `time`, `speed`, `root_motion`; read-only `playing`, `looping`, `duration`, `clips`, `springs`), `transform` (`position`, `rotation`, `scale`), `attr` (attributes by name). `"Name:animation"` gives the component, `"Name"` the entity. A shared name, a wildcard or a wrong key raises `KeyError` with a suggestion; read-only fields raise `AttributeError` |
| `scan(pattern)`, `keys(pattern="*")` | Every value / key matching a `*` / `?` pattern, in scene order, skipping entities without that component or field |
| `ref(key)` | A `ComponentRef` (used like the handle) or a `FieldRef` (`.value`, `get()`, `set(v)`) that looks the key up on each use; keep it from `start` |
| `log(...)`, `warn(...)` | |

Reads see every write made earlier in the frame; writes apply after all
scripts have run. That's the same model as Oberon's Play mode, from the same
module (`automata.script_api`).

**`ScriptCamera`**: `position`, `target`, `fov` (read/assign),
`look_at(point_or_entity)`, and
`follow(entity, offset=(0, 3, 8), smooth=8.0, look_height=0.6)`. Several
writes in one frame accumulate. The same API exists in Oberon's Play mode.
See [Oberon: Cameras](oberon/cameras.md).

### AI helpers (`ctx.ai`, `automata.ai`)

Plain Python, in `automata.script_api` (re-exported as `automata.ai`).
The manual page is [AI](oberon/ai.md).

**`StateMachine(states=None, initial=None, on_change=None)`**: one state
at a time. A state is a function (its update) or a dict/object with
`enter`, `update`, `exit`; each is called with `(entity, ctx)` or
`(entity, ctx, machine)`. `update(entity, ctx)` runs the `when` rules,
then the state's update; a returned state name switches (`exit`,
`on_change(entity, ctx, old, new)`, `enter`). `add(name, spec, enter=,
update=, exit=)`, `when(source_or_"*", target, condition)`, `go(name)`,
`is_in(*names)`, `state`, `previous`, `time` (seconds in the state),
`states`, `history` (last 16). Unknown names raise `KeyError`.

**Behaviour trees**: nodes have `tick(entity, ctx)` (returns `SUCCESS`,
`FAILURE` or `RUNNING`, the strings `"success"`, `"failure"`,
`"running"`), `status`, `reset()`, `describe()`. Functions passed as
children become `Action`s: `True`/`None` is success, `False` failure,
other statuses pass through, anything else raises `ValueError`.

| Node | |
|---|---|
| `Action(fn, name=None)` | `fn(entity, ctx)` |
| `Condition(fn, name=None)` | Success if true, else failure |
| `Sequence(*children)` | In order until one fails; resumes at a running child |
| `Selector(*children)` | In order until one succeeds; re-checks from the first child every tick |
| `Parallel(*children, need="all")` | All every tick; `need="one"` succeeds on the first success |
| `Invert(child)`, `Succeed(child)` | Swap success/failure; always succeed once done |
| `Repeat(child, times=None)` | One repeat per tick; fails if the child fails |
| `Wait(seconds)` | Running until `ctx.time.elapsed` has moved on that much |
| `Cooldown(seconds, child)` | Fails for `seconds` after the child succeeds |

**Senses** (`ctx.ai`, or `AI(ctx)` around any context with `physics.raycast`,
`scene.find` and `time`): `can_see(viewer, target, fov=120, distance=20,
eye=1.6, aim=1.0, forward=(0, 0, -1), walls=True)` (target: an entity, a
name or a point; forward is turned by the viewer's `rotation_euler`; the
line-of-sight ray ignores the viewer); `visible(viewer, targets=None,
...)` (seen entities, nearest first); `noise(position_or_entity,
radius=10, kind="noise", source=None)` returns a `Noise` (`position`,
`radius`, `kind`, `source`, `time`, `distance`) that lasts this frame and
the next; `hear(listener, hearing=1.0, kinds=None)` (noises within
`radius × hearing` not yet heard by this listener, excluding its own);
`noises`.

