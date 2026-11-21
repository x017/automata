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
| Ctrl+E | Go to file (scripts by name, open ones first) |
| Ctrl+Shift+B | Build the game ([Exporting](exporting.md)) |
| Ctrl+, | Settings |
| Ctrl+= / Ctrl+− / Ctrl+0 | Interface zoom in / out / reset |
| Ctrl+wheel over the code editor | Editor zoom only (Settings › Script Editor › Editor scale) |
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
| Ctrl+K Ctrl+C / Ctrl+K Ctrl+U | Comment / uncomment |
| Ctrl+Shift+K | Delete line |
| Alt+↑ / Alt+↓ | Move line up / down |
| Shift+Alt+↑ / Shift+Alt+↓ | Copy line up / down |
| Ctrl+] / Ctrl+[ | Indent / outdent |
| Ctrl+L | Select line |
| Ctrl+Enter / Ctrl+Shift+Enter | New line below / above |
| Ctrl+Shift+Backslash | Jump to the matching bracket |
| Ctrl+↑ / Ctrl+↓ | Scroll without moving the caret |
| Esc | Drop extra carets, then the selection, then leave the editor |

### Multiple carets

Every caret types, deletes, pastes and completes at once, and one Ctrl+Z
undoes the whole edit.

| Keys | Action |
|---|---|
| Ctrl+D | Select the word, then add the next match |
| Ctrl+Shift+L | Select every match |
| Alt+click | Add or remove a caret |
| Ctrl+Alt+↑ / Ctrl+Alt+↓ | Add a caret above / below |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut every selection; pasting as many lines as carets puts one line at each |

## Tabs

These work from the Scene tab too. The Scene is always the first tab.

| Keys | Action |
|---|---|
| Ctrl+Tab / Ctrl+Shift+Tab | Recently used tabs. Hold Ctrl and press Tab again to go further back; release Ctrl to switch |
| Ctrl+PgDn / Ctrl+PgUp | Next / previous tab |
| Alt+1 … Alt+9 | Go to tab by position (Alt+1 is the Scene) |
| Alt+0 | Last tab |
| Ctrl+Shift+PgDn / Ctrl+Shift+PgUp | Move the tab right / left |
| Ctrl+W / Ctrl+F4 | Close the tab |
| Ctrl+Shift+T | Reopen the last closed tab |
| Ctrl+K W / Ctrl+K U | Close all / close saved tabs (unsaved tabs stay open) |
| Ctrl+K S | Save all |

With the mouse: middle-click a tab to close it, drag it to reorder it, and
right-click it for Close Others, Close to the Right, Copy Path and Reveal in
Files. The wheel scrolls the tab strip when tabs overflow.
