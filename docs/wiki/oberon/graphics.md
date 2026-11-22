# Graphics

The **Graphics** tab sits next to the Inspector in the right dock. It
collects everything that decides how the frame looks, plus tools for
testing it. Every edit is undoable and saved with the scene, except
**Quality** and **Preview**, which are editor preferences.

![The Graphics tab beside a scene](../../assets/oberon/graphics.png)

The panel edits real scene entities. Each group shows which entity it
edits, for example *On entity "Sun"*, with a **Select** button that jumps
to that entity in the Inspector. Nothing in the panel is hidden state: a
scene tuned here looks the same in Play and under `at.run_scene`.

## Renderer

- **Quality**: Low / Medium / High / Ultra / Beautiful. This sets the
  shadow map resolution, or turns shadows off on Low. Beautiful switches to
  soft cascaded shadows, physically based light, HDR bloom, FXAA and
  refractive water. It applies immediately.
- **Stats** update live: frame time in ms, fps, meshes drawn, and shadow
  map size. Use them with the *400 cubes* test scene to see what your
  scene costs.

| Quality | Shadows | Shadow map | Glow lights with shadows |
|---|---|---|---|
| Low | off | — | none |
| Medium | on | 1024 px | none |
| High (default) | on | 2048 px | 2 nearest |
| Ultra | on | 4096 px | 4 nearest |
| Beautiful | soft, 4 cascades | 4 × 2048 px | 4 nearest, soft |

Water is a component. Use **Create → Water** (it starts as a calm pond)
or select an entity and use **Add Component → Water** in the Inspector.
It has fields for size, colors, clarity, waves and foam. Tick **Endless**
on a sea to carry it, calm, past its edges to the horizon.
See [Rendering & Lights](../guides/rendering-and-lights.md#water).

## Sun

This group edits the scene's first **Directional Light**. If there isn't
one, **Add Directional Light** creates it.

| Field | Meaning |
|---|---|
| Time of Day | Moves the sun along a simple day arc in one drag: 6 = sunrise, 12 = highest (65°), 18 = sunset. Sets Azimuth and Elevation together |
| Azimuth | Compass direction the light comes **from**, in degrees. 0 = from +Z (behind the default view), 90 = from +X |
| Elevation | Height of the sun above the horizon. 90 = straight down (noon), 5–15 = long sunset shadows |
| Color | Sunlight tint |
| Ambient | How bright the shadowed sides are (0 = black shadows, 1 = no shading) |
| Cast Shadows | Shadow mapping for this light |

Azimuth and elevation are a friendlier way to set the light's direction
vector. The Inspector's Directional Light section shows the same two
fields.

## Environment

The **Environment** component sets the look of the whole scene: the sky
and sun, ambient colour, exposure, tone mapping, and fog. The first entity
that has one wins. A scene without one uses the neutral default look: white
ambient, a near-black background, no fog, and no tone mapping. Click
**Customize Environment** to create an Environment entity and start
tuning.

**Presets** give you a starting point:

| Preset | Look | Tone mapping |
|---|---|---|
| Neutral | Plain lighting, no fog: the default | None |
| Daylight | Blue sky with sun and clouds, warm ground bounce, light haze | ACES |
| Dusk | Violet sky with a big warm sun, dark red ground, dense warm fog | AgX |
| Night | Near-black sky with a small dim disk, cold ambient, black fog | ACES |

| Field | Meaning |
|---|---|
| Sky / Ground | Ambient light from above and from below. Surfaces facing up take the sky colour; surfaces facing down take the ground colour; the rest blend between them. This hemisphere ambient makes flat-coloured scenes look grounded |
| Sky & Sun | Draw a sky dome instead of a flat colour: a gradient from Horizon to Zenith, the sun disk where the Directional Light shines from, a halo around it, and drifting clouds. Water and fog pick up the same sky |
| Background / Horizon | With the sky off, the flat colour behind everything. With it on, the sky colour at the horizon (also the far fog's colour) |
| Zenith | Sky colour straight up |
| Sun Size | Diameter of the sun disk in degrees (the real sun is about 0.5; 2 reads well in games) |
| Sun Glow | Brightness of the halo around the sun. Low suns glow warmer on their own |
| Clouds | Cloud cover, 0 = clear, 1 = overcast |
| Lens Flare | Strength of the glare, starburst and ghosts when you look toward the sun. Beautiful quality only; scenery and clouds in front of the sun hide it |
| Tone Mapping | How bright values are squeezed into the screen's range. **None**: values above 1 clip. **Reinhard**: soft and flat. **ACES**: filmic contrast and saturation. **AgX**: filmic, keeps bright colours from turning neon |
| Exposure | Brightness multiplier applied before tone mapping. Raise it with ACES or AgX, which darken mid-tones |
| Fog | Exponential distance fog |
| Fog Color | Usually the same as Background, so distant objects melt into the sky |
| Fog Density | How quickly fog thickens per metre. 0.01 is light haze, 0.05 is thick |
| Fog Start | Distance from the camera with no fog at all |

> With the sky off, fog is applied before tone mapping and the
> background after it, so with ACES or AgX the far fog can look slightly
> different from the background. If the horizon shows a seam, nudge Fog
> Color, or turn on **Sky & Sun**, which fades fog into the sky itself.

## Camera

This group edits the scene's first camera, the one Play renders through.
If there isn't one, **Add Camera** creates one framed on your current
view.

- **Field of View**
- **Look Through** / **Align to View**
- **Preview**: show the live inset when a camera is selected

Full guide: [Cameras](cameras.md).

## Test Scenes

These buttons drop known content into the scene so you can judge a look
or a setting fairly. Every object they add is named `Test …`, and each
button press is one undo step. **Remove N test objects** deletes them all
at once, also as one undo step.

| Scene | Use it to |
|---|---|
| Color chart | A sweep of hues and brightness. Switch tone mapping and watch how saturated colours and highlights behave |
| Shadows | A floor, pillars, and a floating slab. Check shadow shape, contact, and acne while you move the sun |
| Fog depth | Pillars marching out to 150 m. Tune fog density and start until the far ones fade the way you want |
| 400 cubes | A 20×20 field. Watch the frame time stat at each quality level |
| Lantern | A glowing lantern ringed by crates. Try it at night: on High and up the crates throw shadows outward from the lantern. Untick its **Glow Shadows** to compare |

Test objects are ordinary entities. Remember to remove them before you
save, unless you want to keep them.

## A tuning workflow

1. Add **Shadows** and **Color chart**.
2. Pick the preset closest to your mood.
3. Set **Elevation**: low for drama, high for clarity. Then rotate
   **Azimuth** until the shadows fall where they read best.
4. Adjust **Sky / Ground** until the shadowed sides look right, then
   **Ambient** for overall contrast.
5. Pick **Tone Mapping**, then fix overall brightness with **Exposure**.
6. Turn on **Fog**. Match Fog Color to Background and set **Fog Start**
   just past the play area.
7. Click **Remove test objects** and save.

The same look in plain Python is in
[Guide: Rendering & Lights](../guides/rendering-and-lights.md#environment).
The [Lighting & mood tutorial](../tutorials/lighting-and-mood.md) walks
through three complete looks.
