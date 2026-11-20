"""The Oberon application: a nexa-ui desktop editor for AUTOMATA scenes.

Layout (dark-white sci-fi):

- A menu bar (File / Edit / View) and a play-control toolbar.
- A docked workspace: **Hierarchy** (left), **Viewport** (center),
  **Inspector** (right), **Console** (bottom).
- The viewport is a live software wireframe render of the document; drag to
  orbit, scroll to zoom. The inspector edits the selected entity's
  components. The console logs editor actions and play results.

Run with a display: ``python -m oberon.app`` (or the ``oberon`` script).
The logic lives in sibling modules that never touch nexa, so it stays
headless-testable.
"""

from __future__ import annotations

import sys
import traceback
from typing import Optional

# Allow running both as ``python -m oberon`` and directly as
# ``python oberon/app.py``: in the latter case there is no package context,
# so put the package's parent dir on the path and import it absolutely.
if __package__ in (None, ""):  # direct script run
    import os

    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    __package__ = "oberon"

import nexa
from nexa import Accent, Key, LogBuffer, ThemeMode

from oberon import component_info
from oberon.document import SceneDocument
from oberon.runner import PlayModeRunner
from oberon.viewport import Camera, render_document

_LAYOUT = "h0.20([*Hierarchy,Console],h0.74([*Viewport],[Inspector]))"
_VIEW_W, _VIEW_H = 720, 440


def _color32(rgba) -> "nexa.Color":
    """A nexa Color (0-255 ints) from a 0.0-1.0 tuple."""
    r, g, b = rgba[0], rgba[1], rgba[2]
    a = rgba[3] if len(rgba) > 3 else 1.0
    return nexa.Color(int(r * 255), int(g * 255), int(b * 255), int(a * 255))


def _tuple32(color: "nexa.Color", alpha: float = 1.0) -> tuple:
    """A 0.0-1.0 tuple from a nexa Color."""
    t = tuple(color)
    r, g, b = t[0] / 255.0, t[1] / 255.0, t[2] / 255.0
    a = (t[3] / 255.0) if len(t) > 3 else alpha
    return (r, g, b, a)


class OberonApp:
    def __init__(self, ui: "nexa.UI", scene_path: Optional[str] = None) -> None:
        self.ui = ui
        self.doc = self._load_or_new(scene_path)
        self.runner = PlayModeRunner()
        self.camera = Camera()
        self.log = LogBuffer()
        self.console_open = True
        self.show_grid = True
        self._tex: Optional[int] = None
        self._drag: Optional[tuple] = None
        self._open_dialog = False
        self._save_dialog = False
        self._browse_path = self.doc.path or "."
        self.log.info(f"Oberon ready — editing {self.doc.path or 'untitled scene'}")

    # -- setup / io ---------------------------------------------------------

    def _load_or_new(self, path: Optional[str]) -> SceneDocument:
        if path:
            try:
                return SceneDocument.load(path)
            except Exception as exc:
                print(f"oberon: could not load {path}: {exc}", file=sys.stderr)
        doc = SceneDocument()
        sun = doc.add_entity("Sun")
        doc.add_component(sun.uid, "DirectionalLight")
        cube = doc.add_entity("Cube")
        doc.add_component(cube.uid, "MeshRenderer")
        doc.select(cube.uid)
        doc._dirty = False  # noqa: SLF001 - fresh template starts clean
        doc._undo.clear()  # noqa: SLF001
        return doc

    def _open(self, path: str) -> None:
        try:
            self.doc = SceneDocument.load(path)
            self.log.info(f"opened {path} ({self.doc.entity_count} entities)")
        except Exception as exc:
            self.log.error(f"open failed: {exc}")

    def _save(self) -> None:
        if self.doc.path is None:
            self.log.warn("no scene path yet — use File > Save As")
            return
        try:
            self.doc.save()
            self.log.info(f"saved {self.doc.path}")
        except Exception as exc:
            self.log.error(f"save failed: {exc}")

    def _save_as(self, path: str) -> None:
        try:
            self.doc.save(path)
            self.log.info(f"saved {path}")
        except Exception as exc:
            self.log.error(f"save failed: {exc}")

    # -- per-frame ----------------------------------------------------------

    def draw(self) -> None:
        self._menus()
        self._toolbar()
        with self.ui.dockspace("dock", layout=_LAYOUT):
            self._hierarchy_panel()
            self._viewport_panel()
            self._inspector_panel()
            if self.console_open:
                self._console_panel()
        self._status_bar()
        self._dialogs()
        self._shortcuts()

    # -- file dialogs -------------------------------------------------------

    def _dialogs(self) -> None:
        ui = self.ui
        # Open: a file browser in a modal dialog.
        with ui.dialog("Open Scene", self._open_dialog, width=560.0) as dlg:
            if dlg:
                res = ui.file_browser("open-browser", self._browse_path,
                                      filter=".ron", height=360.0)
                if res.path:
                    self._browse_path = res.path
                if res.picked and res.path:
                    self._open(res.path)
                    dlg.open = False
                if ui.button("Cancel"):
                    dlg.open = False
        self._open_dialog = bool(dlg.open)
        # Save As: a path prompt in a modal dialog.
        with ui.dialog("Save Scene As", self._save_dialog, width=560.0) as sdlg:
            if sdlg:
                ui.text("Save the scene to a .ron path:")
                path = ui.prop_text("Path", self._browse_path, max_bytes=512)
                if path != self._browse_path:
                    self._browse_path = path
                if ui.button("Save"):
                    self._save_as(self._browse_path)
                    sdlg.open = False
                if ui.button("Cancel"):
                    sdlg.open = False
        self._save_dialog = bool(sdlg.open)

    # -- menu bar -----------------------------------------------------------

    def _menus(self) -> None:
        ui = self.ui
        with ui.menu_bar("menubar"):
            with ui.menu_bar_menu("File"):
                if ui.menu_item("New Scene", shortcut="Ctrl+N"):
                    self.doc = SceneDocument()
                    self.log.info("new scene")
                if ui.menu_item("Open…", shortcut="Ctrl+O"):
                    self._browse_path = self.doc.path or "."
                    self._open_dialog = True
                if ui.menu_item("Save", shortcut="Ctrl+S"):
                    if self.doc.path:
                        self._save()
                    else:
                        self._save_dialog = True
                if ui.menu_item("Save As…", shortcut="Ctrl+Shift+S"):
                    self._browse_path = self.doc.path or "scene.ron"
                    self._save_dialog = True
            with ui.menu_bar_menu("Edit"):
                if ui.menu_item("Undo", shortcut="Ctrl+Z", disabled=not self.doc.can_undo()):
                    if self.doc.undo():
                        self.log.info("undo")
                if ui.menu_item("Redo", shortcut="Ctrl+Y", disabled=not self.doc.can_redo()):
                    if self.doc.redo():
                        self.log.info("redo")
                ui.separator()
                if ui.menu_item("Add Entity", shortcut="Ctrl+E"):
                    e = self.doc.add_entity()
                    self.log.info(f"added entity {e.name!r}")
                sel = self.doc.selected()
                if ui.menu_item("Delete Entity", shortcut="Del", disabled=sel is None):
                    if sel is not None:
                        self.doc.delete_entity(sel.uid)
                        self.log.info(f"deleted {sel.name!r}")
            with ui.menu_bar_menu("View"):
                if ui.menu_item("Console", checked=self.console_open):
                    self.console_open = not self.console_open
                if ui.menu_item("Grid", checked=self.show_grid):
                    self.show_grid = not self.show_grid
                ui.separator()
                if ui.menu_item("Reset Camera"):
                    self.camera = Camera()

    # -- toolbar ------------------------------------------------------------

    def _toolbar(self) -> None:
        ui = self.ui
        with ui.toolbar("playbar"):
            playing = self.runner.playing
            if ui.toolbar_button("play", label="Play", disabled=playing,
                                 tooltip="Simulate the scene headless, then restore"):
                self._play()
            if ui.toolbar_button("stop", label="Stop", disabled=not playing):
                self.runner.stop()
                self.log.info("stopped")
            ui.separator()
            secs = ui.prop_float("Sim seconds", float(self.runner.seconds))
            if secs != self.runner.seconds:
                self.runner.seconds = max(0.05, float(secs))
            if self.runner.last_result:
                ui.text(self.runner.last_result, fg=nexa.Token.TEXT_MUTED)

    def _play(self) -> None:
        self.log.info("playing…")
        try:
            self.runner.play(self.doc)
            self.log.info(self.runner.last_result or "play finished")
        except Exception:
            self.log.error("play crashed:\n" + traceback.format_exc(limit=6))
        self.runner.stop()

    # -- hierarchy ----------------------------------------------------------

    def _hierarchy_panel(self) -> None:
        ui = self.ui
        with ui.dock_panel("Hierarchy", icon="list") as panel:
            if not panel:
                return
            if ui.button("+ Add Entity"):
                e = self.doc.add_entity()
                self.log.info(f"added entity {e.name!r}")
            ui.separator()
            if self.doc.entity_count == 0:
                ui.empty_state("Empty scene", description="Add an entity to begin.")
            else:
                for rec in list(self.doc.entities):
                    selected = rec.uid == self.doc.selected_uid
                    label = ("● " if selected else "  ") + (rec.name or "(unnamed)")
                    if ui.button(label):
                        self.doc.select(rec.uid)

    # -- viewport -----------------------------------------------------------

    def _viewport_panel(self) -> None:
        ui = self.ui
        with ui.dock_panel("Viewport", icon="view") as panel:
            if not panel:
                return
            fb = self._render()
            if self._tex is not None:
                ui.image_destroy(self._tex)
            self._tex = ui.image_create(_VIEW_W, _VIEW_H, fb.to_bytes())
            ui.image(self._tex, width=_VIEW_W, height=_VIEW_H)
            self._viewport_input()

    def _render(self):
        """Renders the document; the grid is toggled by ``self.show_grid``."""
        from oberon import viewport as vp
        if self.show_grid:
            return render_document(self.doc, self.camera, _VIEW_W, _VIEW_H,
                                   selected_uid=self.doc.selected_uid)
        saved = vp.ViewportRenderer.draw_grid
        vp.ViewportRenderer.draw_grid = lambda self, fb, **k: None
        try:
            return render_document(self.doc, self.camera, _VIEW_W, _VIEW_H,
                                   selected_uid=self.doc.selected_uid)
        finally:
            vp.ViewportRenderer.draw_grid = saved

    def _viewport_input(self) -> None:
        ui = self.ui
        res = ui.canvas("viewport-input", width=_VIEW_W, height=_VIEW_H)
        mx, my = (res.mouse if res.mouse is not None else (0.0, 0.0))
        if res.pressed:
            if self._drag is None:
                self._drag = (mx, my)
            else:
                dx, dy = mx - self._drag[0], my - self._drag[1]
                self.camera.orbit(dx, dy)
                self._drag = (mx, my)
        else:
            self._drag = None

    # -- inspector ----------------------------------------------------------

    def _inspector_panel(self) -> None:
        ui = self.ui
        with ui.dock_panel("Inspector", icon="sliders") as panel:
            if not panel:
                return
            rec = self.doc.selected()
            if rec is None:
                ui.empty_state("Nothing selected",
                               description="Select an entity in the Hierarchy.")
                return
            self._name_row(rec)
            ui.separator()
            for comp in list(rec.components):
                self._component_group(rec, comp)
            ui.separator()
            self._add_component_row(rec)

    def _name_row(self, rec) -> None:
        ui = self.ui
        new_name = ui.prop_text("Name", rec.name, max_bytes=128)
        if new_name != rec.name and new_name:
            self.doc.rename_entity(rec.uid, new_name)
        ui.text("%d component%s" % (len(rec.components), "" if len(rec.components) == 1 else "s"),
                fg=nexa.Token.TEXT_MUTED)

    def _component_group(self, rec, comp) -> None:
        ui = self.ui
        tname = component_info.type_name_of(comp)
        spec = component_info.spec_for(tname)
        title = spec.title if spec else tname
        is_open, _ = ui.inspector_group(title, is_open=True)
        if not is_open:
            return
        if spec is None or not spec.fields:
            ui.text("(no editable properties)", fg=nexa.Token.TEXT_MUTED)
        else:
            with ui.inspector(f"insp-{rec.uid}-{tname}"):
                for key, label, kind in spec.fields:
                    self._field(rec, comp, tname, key, label, kind)
        if not (spec and spec.required):
            if ui.button(f"Remove {title}"):
                self.doc.remove_component(rec.uid, tname)
                self.log.info(f"removed {title} from {rec.name!r}")

    def _field(self, rec, comp, tname, key, label, kind) -> None:
        ui = self.ui
        value = getattr(comp, key, None)
        if kind == "vec3":
            v = tuple(value) if value is not None else (0.0, 0.0, 0.0)
            out = ui.prop_vec3(label, (float(v[0]), float(v[1]), float(v[2])))
            if tuple(out) != tuple(v):
                self.doc.set_component_field(rec.uid, tname, key, tuple(out))
        elif kind == "vec2":
            v = tuple(value) if value is not None else (0.0, 0.0)
            out = ui.prop_vec3(label, (float(v[0]), float(v[1]), 0.0))
            if (out[0], out[1]) != (v[0], v[1]):
                self.doc.set_component_field(rec.uid, tname, key, (float(out[0]), float(out[1])))
        elif kind == "float":
            out = ui.prop_float(label, float(value or 0.0))
            if out != value:
                self.doc.set_component_field(rec.uid, tname, key, float(out))
        elif kind == "bool":
            out = ui.prop_bool(label, bool(value))
            if out != bool(value):
                self.doc.set_component_field(rec.uid, tname, key, bool(out))
        elif kind in ("color", "rgba"):
            out = ui.prop_color(label, _color32(value if value is not None else (1, 1, 1, 1)))
            newt = _tuple32(out)
            oldt = tuple(value) if value is not None else (1.0, 1.0, 1.0, 1.0)
            target = newt if kind == "rgba" else newt[:3]
            want = oldt if kind == "rgba" else oldt[:3]
            if any(abs(a - b) > 1e-3 for a, b in zip(target, want)):
                self.doc.set_component_field(rec.uid, tname, key, target)
        elif kind == "text":
            out = ui.prop_text(label, str(value if value is not None else ""), max_bytes=128)
            if out != value:
                self.doc.set_component_field(rec.uid, tname, key, out)

    def _add_component_row(self, rec) -> None:
        ui = self.ui
        present = set(rec.component_types())
        choices = [t for t in component_info.ADDABLE if t not in present]
        if not choices:
            return
        ui.text("Add component:", fg=nexa.Token.TEXT_MUTED)
        for t in choices:
            if ui.button(f"+ {t}"):
                if self.doc.add_component(rec.uid, t):
                    self.log.info(f"added {t} to {rec.name!r}")

    # -- console ------------------------------------------------------------

    def _console_panel(self) -> None:
        with self.ui.dock_panel("Console", icon="terminal") as panel:
            if panel:
                self.ui.log_view("console", self.log, height=160.0)

    # -- status bar ---------------------------------------------------------

    def _status_bar(self) -> None:
        ui = self.ui
        with ui.status_bar("status"):
            dirty = " ●" if self.doc.dirty else ""
            name = self.doc.path or "untitled"
            ui.status_bar_text(f"{name}{dirty}")
            ui.status_bar_text(f"{self.doc.entity_count} entities")
            ui.status_bar_text("playing" if self.runner.playing else "editing",
                               tone=nexa.Tone.PRIMARY if self.runner.playing else nexa.Tone.NEUTRAL)

    # -- shortcuts ----------------------------------------------------------

    def _shortcuts(self) -> None:
        ui = self.ui
        mods = ui.modifiers
        ctrl = bool(mods & nexa.Mod.CTRL)
        shift = bool(mods & nexa.Mod.SHIFT)
        if ctrl and ui.key_pressed(Key.S):
            if shift:
                self._browse_path = self.doc.path or "scene.ron"
                self._save_dialog = True
            elif self.doc.path:
                self._save()
            else:
                self._save_dialog = True
        elif ctrl and ui.key_pressed(Key.O):
            self._browse_path = self.doc.path or "."
            self._open_dialog = True
        elif ctrl and ui.key_pressed(Key.N):
            self.doc = SceneDocument()
            self.log.info("new scene")
        elif ctrl and ui.key_pressed(Key.Z) and not shift:
            if self.doc.undo():
                self.log.info("undo")
        elif ctrl and (ui.key_pressed(Key.Y) or (ui.key_pressed(Key.Z) and shift)):
            if self.doc.redo():
                self.log.info("redo")
        elif ctrl and ui.key_pressed(Key.E):
            e = self.doc.add_entity()
            self.log.info(f"added entity {e.name!r}")
        elif ui.key_pressed(Key.DELETE):
            sel = self.doc.selected()
            if sel is not None:
                self.doc.delete_entity(sel.uid)
                self.log.info(f"deleted {sel.name!r}")


# -- entry point -------------------------------------------------------------


def main(argv: Optional[list] = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    scene_path = argv[0] if argv else None
    app_ref: dict = {}

    def draw(ui: "nexa.UI") -> None:
        app = app_ref.get("app")
        if app is None:
            app = OberonApp(ui, scene_path)
            app_ref["app"] = app
        app.draw()

    return nexa.run(
        draw,
        title="Oberon — AUTOMATA Editor",
        width=1440,
        height=860,
        theme=ThemeMode.DARK,
        accent=Accent.BLUE,
    )


if __name__ == "__main__":
    raise SystemExit(main())
