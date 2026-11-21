# Tutorial: Your first scene

**Goal:** build a small level in Oberon, light it, place a camera, play
it, and save it. No code yet.

**You need:** Oberon running (`cargo run -p oberon --release`). See
[Oberon](../oberon/README.md#run-it).

## 1. Start clean

Oberon opens with a starter scene: a sun, a ground, a cube, a player, and
three crates. Choose **File → New Scene** (Ctrl+N) to get an empty one
instead. The Hierarchy on the left is now empty.

## 2. A floor

1. Click **+** at the top of the Hierarchy and choose **Cube**.
2. In the Inspector, set **Transform → Scale** to `20, 0.2, 20` and
   **Position** Y to `-0.1`. Click a field and type, or drag the X / Y /
   Z label left and right to scrub.
3. Click the name field at the top of the Inspector and rename the cube
   `Ground`. F2 also works.
4. Make it solid: choose **Add Component → Rigidbody** and set **Body
   Type** to **Static**. Then choose **Add Component → Collider**; the
   collider sizes itself to the mesh.

(**+ → Plane** makes a flat visual-only quad. That's good for decoration,
but use a cube when things need to stand on it.)

## 3. Some objects

1. **+ → Cube**. Press **W** for the Move gizmo and drag the red arrow to
   move the cube along X.
2. Press **Ctrl+D** a few times to duplicate it, and spread the copies
   around.
3. Select one cube and click the **Color** swatch under **Mesh Renderer**.
   Pick a colour, or type a hex value such as `#E8613A`.
4. Press **R** for Scale and stretch one cube into a tall pillar.

Made a mistake? **Ctrl+Z** undoes it. Every edit is one step, including a
whole gizmo drag.

## 4. Light

Choose **+ → Directional Light**. A sun icon appears in the view, and the
objects get light and shadows.

Open the **Graphics** tab (right dock, beside Inspector). Under **Sun**:
- drag **Elevation** down to about `25` for long evening shadows
- drag **Azimuth** around until the shadows fall where you like them

## 5. A player

Choose **+ → Player**. You get an orange capsule-sized box with a
**Character Controller**. In Play mode it walks with WASD, jumps with
Space, and sprints with Shift. No script is needed for that.

Move it up a little so it isn't stuck inside the floor. A Y position of
`0.9` stands it on the ground.

## 6. A camera

Orbit the view (drag empty space) and zoom (wheel) until you have a nice
shot from behind and above the player. Then choose **+ → Camera**.

The new camera is placed exactly where you're looking from:

- a **frustum** pyramid shows what it sees
- a **preview** in the bottom-right corner shows its live view

Orbit away and the frustum stays put. Click **Look Through** in the
Inspector to jump back into the camera's view. Adjust the shot, then
click **Align to View** to move the camera there.

## 7. Play

Press **▶** (Ctrl+P).

- The view switches to your camera.
- WASD moves the player relative to the camera, and Space jumps.
- Press **■** to stop. Everything returns to exactly where it was.

The camera doesn't follow the player yet; you'll script that in
[Third-person camera](third-person-camera.md). If you delete the camera,
Play falls back to the editor view following the player.

## 8. Save

**Ctrl+S** asks for a file name. Save as `my_game/level.ron` (make a new
folder for it). The title bar shows the name, and the unsaved-changes dot
disappears.

The file is plain text you can read. Open it from **Project**, or in any
editor:

```ron
(
    version: 1,
    entities: [
        (
            name: Some("Ground"),
            transform: Some((position: (0.0, 0.0, 0.0), scale: (20.0, 1.0, 20.0), ...)),
            ...
```

## What you learned

- Hierarchy **+** creates objects; the Inspector edits them.
- W / E / R gizmos, Ctrl+D duplicate, Ctrl+Z undo.
- The sun's direction lives in Graphics → Sun.
- A Camera decides what Play shows, and Look Through / Align to View
  frame it.
- Scenes are `.ron` text files.

**Next:** [Coin collector](coin-collector.md). Make it a game with Python.
