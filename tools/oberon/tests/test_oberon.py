"""Headless tests for Oberon's logic layer (no nexa, no display)."""

from __future__ import annotations

import automata as at
import pytest

from oberon.document import SceneDocument
from oberon.runner import PlayModeRunner, PlayState
from oberon.viewport import Camera, FrameBuffer, render_document


# -- document ---------------------------------------------------------------


def test_new_document_starts_empty_and_clean():
    doc = SceneDocument()
    assert doc.entity_count == 0
    assert not doc.dirty
    assert doc.selected() is None


def test_add_entity_selects_and_marks_dirty():
    doc = SceneDocument()
    rec = doc.add_entity("Cube")
    assert doc.entity_count == 1
    assert doc.dirty
    assert doc.selected_uid == rec.uid
    assert rec.find("Transform") is not None  # default transform staged


def test_unique_names():
    doc = SceneDocument()
    a = doc.add_entity("Entity")
    b = doc.add_entity()
    assert b.name != a.name


def test_add_component_and_guard_duplicates():
    doc = SceneDocument()
    rec = doc.add_entity("Cube")
    assert doc.add_component(rec.uid, "MeshRenderer")
    assert not doc.add_component(rec.uid, "MeshRenderer")  # duplicate rejected
    assert rec.find("MeshRenderer") is not None


def test_remove_component_but_not_transform():
    doc = SceneDocument()
    rec = doc.add_entity("Cube")
    doc.add_component(rec.uid, "MeshRenderer")
    assert doc.remove_component(rec.uid, "MeshRenderer")
    assert not doc.remove_component(rec.uid, "Transform")  # required


def test_set_component_field_edits_value():
    doc = SceneDocument()
    rec = doc.add_entity("Cube")
    doc.set_component_field(rec.uid, "Transform", "position", (1.0, 2.0, 3.0))
    assert rec.find("Transform").position == (1.0, 2.0, 3.0)


def test_frozen_component_field_edit_rebuilds():
    doc = SceneDocument()
    rec = doc.add_entity("Sun")
    doc.add_component(rec.uid, "DirectionalLight")
    ok = doc.set_component_field(rec.uid, "DirectionalLight", "ambient", 0.9)
    assert ok
    assert abs(rec.find("DirectionalLight").ambient - 0.9) < 1e-6


def test_save_load_round_trip(tmp_path):
    doc = SceneDocument()
    cube = doc.add_entity("Cube")
    doc.add_component(cube.uid, "MeshRenderer")
    doc.add_component(cube.uid, "RigidBody")
    doc.set_component_field(cube.uid, "Transform", "position", (2.0, 0.0, 0.0))
    path = str(tmp_path / "scene.ron")
    doc.save(path)
    assert not doc.dirty

    loaded = SceneDocument.load(path)
    assert loaded.entity_count == 1
    rec = loaded.entities[0]
    assert rec.name == "Cube"
    assert rec.find("MeshRenderer") is not None
    assert rec.find("RigidBody") is not None
    assert rec.find("Transform").position == (2.0, 0.0, 0.0)


def test_undo_redo_restores_state():
    doc = SceneDocument()
    rec = doc.add_entity("Cube")
    doc.set_component_field(rec.uid, "Transform", "position", (5.0, 0.0, 0.0))
    assert rec.find("Transform").position == (5.0, 0.0, 0.0)
    assert doc.undo()
    assert doc.find(rec.uid).find("Transform").position == (0.0, 0.0, 0.0)
    assert doc.redo()
    assert doc.find(rec.uid).find("Transform").position == (5.0, 0.0, 0.0)


def test_undo_add_entity_removes_it():
    doc = SceneDocument()
    doc.add_entity("Cube")
    assert doc.entity_count == 1
    doc.undo()
    assert doc.entity_count == 0


# -- viewport ---------------------------------------------------------------


def test_framebuffer_dimensions_and_bytes():
    fb = FrameBuffer(64, 32)
    assert fb.width == 64 and fb.height == 32
    assert len(fb.to_bytes()) == 64 * 32 * 4


def test_render_document_produces_nonblank_frame():
    doc = SceneDocument()
    cube = doc.add_entity("Cube")
    doc.add_component(cube.uid, "MeshRenderer")
    fb = render_document(doc, Camera(), 160, 120, selected_uid=cube.uid)
    # the grid + mesh should have lit at least some non-background pixels
    bg = bytes((18, 19, 24, 255))
    px = fb.to_bytes()
    lit = sum(1 for i in range(0, len(px), 4) if px[i:i+4] != bg)
    assert lit > 50


# -- runner -----------------------------------------------------------------


def test_play_simulates_and_restores():
    doc = SceneDocument()
    cube = doc.add_entity("Cube")
    doc.set_component_field(cube.uid, "Transform", "position", (0.0, 0.0, 0.0))

    def tick(ctx):
        e = ctx.scene.find("Cube")
        if e and e.transform:
            x, y, z = e.transform.position
            e.transform.position = (x + 1.0, y, z)

    runner = PlayModeRunner()
    runner.set_update(tick)
    runner.play(doc, seconds=0.2)  # ~12 frames
    assert runner.last_result is not None
    assert "frames" in runner.last_result
    # play-mode changes are discarded: the document is back to the snapshot
    assert doc.find(cube.uid).find("Transform").position == (0.0, 0.0, 0.0)
    assert runner.state is PlayState.DONE
    runner.stop()
    assert runner.state is PlayState.EDITING


def test_play_without_script_still_runs():
    doc = SceneDocument()
    doc.add_entity("Cube")
    runner = PlayModeRunner()
    runner.play(doc, seconds=0.1)
    assert runner.last_result is not None
    assert runner.state is PlayState.DONE
