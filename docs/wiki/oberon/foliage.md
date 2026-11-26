# Grass & foliage

Oberon paints grass and scattered models (rocks, flowers, trees) straight
onto the ground with a brush. What the components do and how they look
is in the [Grass & Foliage guide](../guides/grass-and-foliage.md). This
page is about painting.

## Grass

**Create → Grass** makes an entity with a **Grass** component, filled
over a 10 × 10 m area. To start from bare ground and only paint, set
**Area Width** and **Area Depth** to 0.

| Field | Meaning |
|---|---|
| Area Width / Area Depth | The filled rectangle, metres (0 for none) |
| Density | Blades per square metre where fully painted |
| Height / Height Vary | The tallest blades, and how much shorter some are |
| Blade Width | Blade width at the root |
| Root Color / Tip Color | The blades' colours, root to tip |
| Color Vary | How much blades' colours differ |
| Draw Distance | How far from the camera it's drawn |
| Seed | Picks which random field grows |

## Scatter

Add a **Scatter** component (Add Component → Scatter) to any entity with
a **Mesh Renderer**. Its model is then drawn at many spots instead of
once. Use a small, low-poly model: every copy costs its triangles.

| Field | Meaning |
|---|---|
| Area Width / Area Depth | The filled rectangle, metres |
| Density | Copies per square metre where fully painted |
| Scale Min / Scale Max | The range of sizes |
| Follow Slope | 0 = upright, 1 = tilted with the ground |
| Seed | Picks where the copies go |

## The brush

Select the entity. Under its Grass or Scatter section in the Inspector:

1. Click **Paint** (or **Erase**). The brush is on, and the move and
   rotate gizmo steps aside while it is.
2. Pick a **Size**: 0.5, 1.5, 3 or 6 m across the disc.
3. Pick a **Strength**: 25 %, 50 % or 100 %. Lower strengths paint
   thinner grass, or erase only part of it.
4. Drag over the ground in the scene view. A ring shows where the brush
   will land. It sits on whatever surface is under the cursor (terrain,
   rocks, a roof) and falls back to the ground plane at height 0.
   Grass and scattered models painted over a [Terrain](terrain.md)
   stand on it, and follow it when it's sculpted more.

Each drag is **one undo step** (Ctrl+Z). Click Paint or Erase again to
turn the brush off. It's also off during Play.

Strokes are stored on the component, in the entity's own space, so
moving the entity carries what you painted with it. The note under the
brush counts the strokes. Painted grass sways in the Environment's
**Wind**.

## Wind

The **Environment** has two rows for the wind that blows through the
whole scene:

| Field | Meaning |
|---|---|
| Wind | 0 still to 1 a gale. Grass sways, spring bones stream out, and water throws spray from 0.5 up |
| Wind Direction | Which way it blows, degrees around Y (0 = toward +X) |

A **Water** component's own **Wind** row still works on its own; spray
follows whichever wind is stronger.

## Try it

Open `examples/oberon/meadow/level.ron`:

- Select **Meadow**, click **Erase** with the 1.5 m brush, and cut a new
  path. Press Play and walk it (W A S D, Shift to run).
- Select **Poppies**, click **Paint** and add a bed by the trees.
- Set the Environment's **Wind** to 1 and watch the field, her ponytail
  and skirt all go the same way.
