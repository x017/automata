# The interface

```text
┌ File Edit GameObject Component Window Help ─── ▶ ❚❚ ■ ───── Search ⚙ ┐
│ Hierarchy    │ Scene  script.py               │ Inspector  Graphics  │
│              │                                │                      │
│  Sun         │      (3D viewport / editor)    │  name, components,   │
│  Ground      │                                │  attributes          │
│  Main Camera │                                │                      │
├ Files ───────┤                                │                      │
│ ▾ scripts    ├────────────────────────────────┤                      │
│     coin.py  │ Console                        │                      │
└ status bar ──────────────────────────────────────────── shortcut hints ┘
```

All panel borders can be dragged. **Window → Reset Layout** restores the
default sizes.

## Top bar

- **Menus**: File (scenes, scripts, prefabs), Edit (undo, clipboard,
  tools), GameObject (create), Component (add to the selection), Window,
  Help.
- **Play / Pause / Stop** sit in the centre. The scene name next to the
  menus shows a dot when you have unsaved changes.
- **Search** (Ctrl+Shift+P) opens the command palette. Type part of any
  command, entity, or script name and press Enter.
- **⚙** (Ctrl+,) opens Settings.

## Hierarchy

Lists every entity in the scene. Each icon shows the entity's main
component: cube, sun, camera, horizon (Environment), or empty.

- Click to select. Double-click (or press F) to frame the entity in the
  view.
- **+** opens the Create menu: Empty, Cube, Plane, Directional Light,
  Camera, Environment, Physics Cube, Player.
- Right-click an entity for Rename, Duplicate, Copy/Paste, Delete, Frame,
  Save as Prefab, and Attach/Edit Script.

## Files

The bottom of the left sidebar is a file tree of your project: the folder
your scene is saved in. Drag the divider between Hierarchy and Files to
share the space, or hide either one from the **Window** menu.

- Click a folder to expand or collapse it. The **collapse** button in the
  panel's title bar folds everything back up.
- Double-click a `.py` file to open it in the code editor, a `.ron` file
  to open it as a scene, or a `.prefab` to place it in the scene.
- The open scene and the script you're editing are shown in bright text
  with a dot.
- Right-click a file for Open, Edit Script, **Attach to** the selected
  entity, or Instantiate. Right-click anywhere for **New Script…** and
  **New Folder**. New files go in the selected folder, and the tree opens
  to show them.
- **+** in the title bar does the same as right-clicking empty space.

Hidden files and `__pycache__`, `target`, and `node_modules` folders are
left out.

## Scene view

This is the 3D viewport, rendered by the engine's own wgpu renderer.

| To | Do |
|---|---|
| Select | Click an object |
| Orbit | Drag empty space, right-drag, or Alt+drag |
| Pan | Middle-drag |
| Zoom | Mouse wheel |
| Frame selection | F, or double-click in the hierarchy |
| Context menu | Right-click without dragging |

**Gizmos.** The floating toolbar (top-left) switches between **Move** (W),
**Rotate** (E), and **Scale** (R). Drag a coloured axis handle to edit
along that axis. Arrow keys nudge the selection across the ground relative to the view, PgUp/PgDn move it up and down, and holding Shift makes the steps finer. The axis
widget (top-right) shows the view orientation.

**Overlays.**
- Lights draw as a small sun icon.
- Cameras draw their view **frustum**: a pyramid showing exactly what the
  camera sees, with a notch marking "up".
- When a camera is selected, a **live preview** of its view appears in
  the bottom-right corner. See [Cameras](cameras.md).

## Inspector

Shows the selected entity.

- **Name** at the top.
- **Components**: Transform, Mesh Renderer, Directional Light, Camera,
  Environment, Rigidbody, Collider, Character Controller, Script. Each
  section folds open and closed. The **⋮** menu has Reset, Copy/Paste
  Component, and Remove.
- **Fields**: drag the label to scrub a number, or click the field and
  type. Colour swatches open a colour picker with hex input.
- **Attributes**: custom values your scripts read. See
  [Prefabs & attributes](prefabs-and-attributes.md).
- **Add Component** lists every component the entity doesn't have yet.

Every edit is one undo step. Scrubbing a value counts as one step, not
one per frame.

## Graphics

The second tab in the right dock. It's a single place to test and tune how
the scene looks:
- renderer quality and live frame time
- the sun angle
- environment presets, tone mapping, and fog
- the scene camera
- one-click test scenes

Full guide: [Graphics](graphics.md).

![The Graphics tab](../../assets/oberon/graphics.png)

## Console

The bottom dock collects `print()` output from your scripts, Oberon
messages, and Python errors with clickable file and line links. The
Info / Warnings / Errors chips filter the list.

## Play mode

Press **▶** (Ctrl+P). Oberon copies the scene and starts simulating:

- **Physics.** Rigidbodies fall and collide. The **Player** (Character
  Controller) walks with WASD or arrows, jumps with Space, and sprints
  with Shift.
- **Scripts** get `start` once, then `update` every frame.
- **Camera.** If the scene has a Camera, Play renders through it, and
  player input is relative to that camera's direction. Without one, the
  editor view follows the player.
- **❚❚** pauses. **■** (Ctrl+P again) stops and restores the scene
  exactly as it was before Play.

Edits are locked during Play, so you can't lose work to a scene that gets
thrown away on Stop.

## Status bar

The left side shows the entity count and the current selection. The right
side shows shortcut hints for whatever has focus. You can turn the hints
off in **Settings → Appearance**.
