# The code editor

Oberon has a native code editor built in, so you can write scripts without
leaving the scene. Scripts open as tabs next to the **Scene** tab. Press
**Esc** to go back to the scene view; the tab stays open.

![Editing a script with find](../../assets/oberon/code.png)

## Open a script

- Double-click a `.py` file in **Files** (left sidebar).
- **File → New Script…** / **Open Script…**.
- Right-click an entity → **Edit Script**.
- Click a **file:line** link in the Console.
- Search for the file name in the command palette (Ctrl+Shift+P).

## Editing

You get what you'd expect from a modern editor:
- Python syntax highlighting
- auto-indent after `:`
- bracket and quote pairing
- line moves, duplicates, and comments
- indentation guides
- current-line highlight
- multi-line paste that keeps relative indentation

The full key list is in [Keyboard shortcuts](shortcuts.md#code-editor).

**Find / Replace** (Ctrl+F / Ctrl+H) supports case-sensitive and
whole-word matching, shows match counts, and can replace all at once.

The **status line** shows the cursor position, selection size, diagnostics
counts, indentation, encoding, and line endings.

## Completion

Completion is built in and needs no setup. It knows the scripting API:

- `ctx.` → `time`, `input`, `scene`, `camera`, `physics`, `log`, …
- `entity.transform.` → `position`, `rotation_euler`, `scale`, …
- `ctx.camera.` / `entity.camera.` → `position`, `target`, `fov`,
  `look_at`, `follow`
- inside `ctx.input.pressed("`: every key name
- inside `ctx.scene["`: the names of the entities in your scene
- inside `entity.attributes["`: the attribute names you've defined

Suggestions appear as you type. Turn that off in **Settings → Script
Editor** if you'd rather press **Ctrl+Space**. Hover a name for its
documentation.

## Language servers

If a Python language server is installed, Oberon starts it when you open
a script. It adds:
- diagnostics (squiggles plus the status-line counts)
- richer completion
- hover
- **F12** go to definition

Oberon looks for these, in order: **basedpyright**, **pyright**,
**pylsp**, **jedi-language-server**.

```console
pip install basedpyright        # recommended
```

**Settings → Python → Language server** shows which one is running and
has a **Restart server** button.

## Settings

**Settings → Script Editor** has these options:
- font size
- current-line highlight
- indentation guides
- whitespace markers
- suggest while typing
- trim trailing whitespace on save
- **Save scripts before Play**, on by default, so Play always runs what
  you see

Prefer another editor? Edit the files in VS Code, Vim, or anything else.
Oberon hot-reloads saved scripts during Play either way.
