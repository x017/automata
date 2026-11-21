# Keyboard shortcuts

Press **F1** inside Oberon to see this list in the app. On macOS, Ctrl
means ⌘.

## Everywhere

| Keys | Action |
|---|---|
| Ctrl+N | New scene |
| Ctrl+O | Open scene |
| Ctrl+S | Save scene (or the focused script) |
| Ctrl+Shift+S | Save scene as |
| Ctrl+P | Play / Stop |
| Ctrl+Shift+P | Command palette |
| Ctrl+Shift+B | Build the game ([Exporting](exporting.md)) |
| Ctrl+, | Settings |
| Ctrl+= / Ctrl+− / Ctrl+0 | Interface zoom in / out / reset |
| Ctrl+Q | Quit |
| F1 | Keyboard shortcuts |

## Scene

| Keys | Action |
|---|---|
| Click | Select |
| Drag, right-drag, Alt+drag | Orbit |
| Middle-drag | Pan |
| Wheel | Zoom |
| F | Frame the selection |
| W / E / R | Move / Rotate / Scale gizmo |
| Arrows | Nudge the selection across the ground, relative to the view |
| PgUp / PgDn | Nudge the selection up / down |
| Shift + nudge | Fine steps |
| Ctrl+Z / Ctrl+Y | Undo / Redo |
| Ctrl+C / Ctrl+V | Copy / Paste entity |
| Ctrl+D | Duplicate |
| Del | Delete |
| F2 | Rename |
| Esc | Deselect |

## Play mode (default player)

| Keys | Action |
|---|---|
| W A S D or arrows | Move (relative to the camera) |
| Space | Jump |
| Shift | Sprint |

Your scripts can read any key: `ctx.input.pressed("q")`.

## Code editor

| Keys | Action |
|---|---|
| Ctrl+Space | Complete |
| F12 | Go to definition |
| Ctrl+F / Ctrl+H | Find / Replace |
| Ctrl+G | Go to line |
| Ctrl+/ | Toggle comment |
| Ctrl+D | Duplicate line |
| Ctrl+Shift+K | Delete line |
| Alt+↑ / Alt+↓ | Move line up / down |
| Ctrl+] / Ctrl+[ | Indent / outdent |
| Ctrl+L | Select line |
| Ctrl+Enter | New line below |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+W | Close tab |
| Esc | Back to the scene view |
