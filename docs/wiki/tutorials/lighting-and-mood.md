# Tutorial: Lighting & mood

**Goal:** give the same level three very different moods (bright
afternoon, golden dusk, and moonlit night) using only Oberon's
**Graphics** tab. Then reproduce one of them in plain Python.

**You'll learn:**
- the sun's angles
- hemisphere ambient
- tone mapping and exposure
- fog
- using test scenes to judge a look fairly

**Start from:** any scene with some objects, such as the coin collector.

## 0. Set up a fair test

Open the **Graphics** tab and, under **Test Scenes**, click **Color
chart** and **Shadows**.
- The chart shows how every hue reacts to your settings.
- The pillars show shadow direction and contrast.

They're named `Test …`, so you'll remove them all at the end with one
click.

Under **Renderer**, set Quality to **High** and note the frame time.

## 1. Bright afternoon

1. **Environment → Customize Environment**, then the **Daylight**
   preset.
2. **Sun:**
   - **Elevation** `55`: high sun, short shadows, clear shapes.
   - **Azimuth** `35`: slightly from the side, so cubes show two
     different shades.
   - **Color** `#FFF5E0`: barely warm.
   - **Ambient** `0.3`: shadows stay clearly darker than lit ground.
3. **Environment:**
   - **Tone Mapping**: ACES.
   - **Exposure**: `1.1`.

   Look at the colour chart: saturated reds and blues stay rich, and
   highlights roll off instead of clipping.
4. **Fog** on, **Fog Density** `0.01`, **Fog Start** `15`. Distant
   objects pick up a gentle haze, which adds depth.

## 2. Golden dusk

1. Click the **Dusk** preset.
2. **Sun:**
   - **Elevation** `8`: long dramatic shadows.
   - **Azimuth**: rotate until shadows stretch across the play area
     toward the camera. With the camera behind the player that's about
     `200`, a low sun from the far side of the level.
   - **Color** `#FFB36B`: deep orange.
   - **Ambient** `0.3`.
3. Notice the ambient: the **Sky** is violet and the **Ground** a dark
   red. Faces pointing up get the cool sky, and the undersides get warm
   bounce. That colour contrast is most of what makes a scene feel like
   sunset.
4. **Tone Mapping** is AgX. Compare it with ACES on the colour chart:
   AgX keeps the bright orange from turning neon yellow.
5. Click **Fog depth** under Test Scenes. Raise **Fog Density** until the
   farthest pillars just disappear (about `0.03`), and set **Fog Start**
   so the play area stays crisp.

## 3. Moonlit night

1. Click the **Night** preset.
2. **Sun** (it's the moon now):
   - **Elevation** `40`, **Azimuth** `320`.
   - **Color** `#9DB4FF`: cold blue.
   - **Ambient** `0.2`.
3. Night scenes fail when everything is just dark. Keep **contrast**:
   - The moon stays fairly bright, so lit faces read.
   - Ambient stays low, so shadows are deep.
   - Fog is black and close (**Fog Start** `2`), so the world falls away
     into darkness.
4. To make it moodier, lower **Exposure** to `0.8`. Don't lower the
   moon's colour, which loses the shape of objects.

## 4. Check performance

Click **400 cubes**. Switch Quality between Low, Medium, High, and Ultra
and watch the frame time in **Renderer**. The shadow map is the main cost.
Medium is usually indistinguishable from High at a distance.

## 5. Clean up and save

Under Test Scenes, click **Remove N test objects**, then **Ctrl+S**. The
look is stored in the scene's **Environment** and **Directional Light**
entities. Select them in the Hierarchy and the Inspector shows the same
fields.

To keep all three looks, use **File → Save As…** after each one, for
example `level.ron`, `level_dusk.ron` and `level_night.ron`.

**Finished files:** the example project ships all three:
[`level.ron`](../../../examples/oberon/coin_run/level.ron) (afternoon),
[`level_dusk.ron`](../../../examples/oberon/coin_run/level_dusk.ron) and
[`level_night.ron`](../../../examples/oberon/coin_run/level_night.ron).
Open any of them in Oberon and look at the Graphics tab.

## 6. The same look in Python

Everything above is data, so code-first games get the same result:

```python
import automata as at

scene = at.Scene()
scene.add(
    at.Entity("moon").add(
        at.DirectionalLight(direction=(-0.3, -0.64, -0.7), color=(0.62, 0.7, 1.0), ambient=0.2)
    )
)
scene.add(
    at.Entity("look").add(
        at.Environment(
            sky_color=(0.25, 0.3, 0.5),
            ground_color=(0.05, 0.05, 0.08),
            background=(0.008, 0.01, 0.02),
            exposure=0.8,
            tonemapping="aces",
            fog=True,
            fog_color=(0.008, 0.01, 0.02),
            fog_density=0.05,
            fog_start=2.0,
        )
    )
)
```

Or start from a preset with `at.Environment.preset("night")`. Environment
values are read-only, so to change a few fields of a preset, copy it into a
new one. This is how
[`build_level.py`](../../../examples/oberon/coin_run/build_level.py) makes
the three looks:

```python
FIELDS = ("sky_color", "ground_color", "background", "exposure", "tonemapping",
          "fog", "fog_color", "fog_density", "fog_start")


def environment(preset, **changes):
    base = at.Environment.preset(preset)
    return at.Environment(**{**{f: getattr(base, f) for f in FIELDS}, **changes})


dusk = environment("dusk", tonemapping="agx", fog_density=0.03, fog_start=8.0)
```

> **Direction from angles:** the light travels *toward*
> `-(cos(el)·sin(az), sin(el), cos(el)·cos(az))`, where `az` is Azimuth
> and `el` is Elevation in radians.

## Cheat sheet

| Want | Change |
|---|---|
| More drama | Lower Elevation; lower Ambient |
| Softer, friendlier | Higher Ambient; Sky and Ground closer together |
| Depth | Fog with Fog Color = Background |
| Punchy colour | ACES |
| Bright colours without neon | AgX |
| Everything too dark after tone mapping | Raise Exposure, not the sun |
| Flat-looking cubes | Azimuth off-axis, so neighbouring faces differ |

**Next:** [From Oberon to Python](oberon-to-python.md).
