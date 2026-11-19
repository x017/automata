"""The Oberon scene document model.

A pure-Python, nexa-free model of the scene being edited. It mirrors the
engine's staged-scene representation (flat list of named entities, each
with a list of component value objects) and adds editor state on top:

- stable per-entity ids (so selection survives list rebuilds),
- a current selection,
- dirty tracking + an on-disk path,
- a small undo stack of RON snapshots,
- save/load round-trips through ``automata.Scene``.

The engine's ``Scene``/``Entity`` builders are write-only (you can add but
not read components back), so Oberon keeps its own parallel record of the
component objects it stages. Components are rebuilt (fresh value objects)
on every edit, so the model never holds a stale handle into the engine.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

import automata as at

from . import component_info

_UNDO_LIMIT = 100


def _clone_component(comp: Any, overrides: Optional[Dict[str, Any]] = None) -> Any:
    """A fresh component value like ``comp``, with ``overrides`` applied.

    Engine pyclasses are not picklable and frozen ones reject attribute
    writes, so we rebuild from the spec's fields at construction time —
    substituting any override — rather than mutating a copy.
    """
    tname = component_info.type_name_of(comp)
    cls = type(comp)
    spec = component_info.spec_for(tname)
    kwargs: Dict[str, Any] = {}
    if spec is not None:
        for key, _label, _kind in spec.fields:
            try:
                kwargs[key] = getattr(comp, key)
            except AttributeError:
                pass
    if overrides:
        kwargs.update(overrides)
    try:
        return cls(**kwargs) if kwargs else cls()
    except TypeError:
        try:
            return cls()
        except TypeError:
            return component_info.make_component(tname)


@dataclass
class EntityRecord:
    """One entity in the document: an id, a name, and component values."""

    uid: int
    name: str
    #: Fresh component value objects (e.g. ``at.Transform(...)``), in the
    #: order they were staged. Rebuilt on edit; never engine-borrowed.
    components: List[Any] = field(default_factory=list)

    def component_types(self) -> List[str]:
        return [component_info.type_name_of(c) for c in self.components]

    def find(self, type_name: str) -> Optional[Any]:
        for c in self.components:
            if component_info.type_name_of(c) == type_name:
                return c
        return None


class SceneDocument:
    """An editable scene: entities, selection, dirty flag, undo history."""

    def __init__(self) -> None:
        self._entities: List[EntityRecord] = []
        self._next_uid = 1
        self.path: Optional[str] = None
        self.selected_uid: Optional[int] = None
        self._dirty = False
        self._undo: List[str] = []
        self._redo: List[str] = []

    # -- queries -----------------------------------------------------------

    @property
    def entities(self) -> List[EntityRecord]:
        return self._entities

    @property
    def entity_count(self) -> int:
        return len(self._entities)

    @property
    def dirty(self) -> bool:
        return self._dirty

    def find(self, uid: int) -> Optional[EntityRecord]:
        for e in self._entities:
            if e.uid == uid:
                return e
        return None

    def selected(self) -> Optional[EntityRecord]:
        if self.selected_uid is None:
            return None
        return self.find(self.selected_uid)

    # -- mutation helpers ---------------------------------------------------

    def _snapshot(self) -> str:
        """The current scene as RON (the undo unit)."""
        return self.to_ron()

    def _push_undo(self) -> None:
        self._undo.append(self._snapshot())
        if len(self._undo) > _UNDO_LIMIT:
            self._undo.pop(0)
        self._redo.clear()

    def _mark_dirty(self) -> None:
        self._dirty = True

    def _alloc_uid(self) -> int:
        uid = self._next_uid
        self._next_uid += 1
        return uid

    def add_entity(self, name: Optional[str] = None, record_undo: bool = True) -> EntityRecord:
        """Adds an entity with a default Transform and selects it."""
        if record_undo:
            self._push_undo()
        if name is None:
            name = self._unique_name("Entity")
        rec = EntityRecord(uid=self._alloc_uid(), name=name, components=[at.Transform()])
        self._entities.append(rec)
        self.selected_uid = rec.uid
        self._mark_dirty()
        return rec

    def delete_entity(self, uid: int) -> bool:
        for i, e in enumerate(self._entities):
            if e.uid == uid:
                self._push_undo()
                self._entities.pop(i)
                if self.selected_uid == uid:
                    self.selected_uid = None
                self._mark_dirty()
                return True
        return False

    def rename_entity(self, uid: int, name: str) -> bool:
        rec = self.find(uid)
        if rec is None or not name:
            return False
        self._push_undo()
        rec.name = name
        self._mark_dirty()
        return True

    def add_component(self, uid: int, type_name: str) -> bool:
        rec = self.find(uid)
        if rec is None:
            return False
        if rec.find(type_name) is not None:
            return False  # one of each type per entity
        self._push_undo()
        rec.components.append(component_info.make_component(type_name))
        self._mark_dirty()
        return True

    def remove_component(self, uid: int, type_name: str) -> bool:
        rec = self.find(uid)
        if rec is None:
            return False
        spec = component_info.spec_for(type_name)
        if spec is not None and spec.required:
            return False
        for i, c in enumerate(rec.components):
            if component_info.type_name_of(c) == type_name:
                self._push_undo()
                rec.components.pop(i)
                self._mark_dirty()
                return True
        return False

    def set_component_field(self, uid: int, type_name: str, key: str, value: Any) -> bool:
        """Edits one field of one component on one entity (records undo)."""
        rec = self.find(uid)
        if rec is None:
            return False
        comp = rec.find(type_name)
        if comp is None:
            return False
        self._push_undo()
        try:
            setattr(comp, key, value)
        except (AttributeError, TypeError):
            # Frozen pyclass (e.g. DirectionalLight): rebuild a copy with the
            # edit applied at construction and swap it in.
            new_comp = _clone_component(comp, {key: value})
            idx = rec.components.index(comp)
            rec.components[idx] = new_comp
        self._mark_dirty()
        return True

    # -- selection ----------------------------------------------------------

    def select(self, uid: Optional[int]) -> None:
        self.selected_uid = uid

    # -- serialization ------------------------------------------------------

    def _unique_name(self, base: str) -> str:
        existing = {e.name for e in self._entities}
        if base not in existing:
            return base
        i = 2
        while f"{base} {i}" in existing:
            i += 1
        return f"{base} {i}"

    def to_scene(self) -> "at.Scene":
        """Builds an engine ``Scene`` from the document."""
        scene = at.Scene()
        for rec in self._entities:
            entity = at.Entity(rec.name)
            for comp in rec.components:
                entity.add(comp)
            scene.add(entity)
        return scene

    def to_ron(self) -> str:
        return self.to_scene().to_ron()

    def save(self, path: Optional[str] = None) -> str:
        """Writes the scene to ``path`` (or the current path). Returns it."""
        target = path or self.path
        if target is None:
            raise ValueError("no scene path set; pass one to save()")
        with open(target, "w", encoding="utf-8") as fh:
            fh.write(self.to_ron())
        self.path = target
        self._dirty = False
        return target

    @classmethod
    def load(cls, path: str) -> "SceneDocument":
        """Loads a scene file into a new document."""
        scene = at.Scene.load(path)
        doc = cls()
        for entity in scene.entities():
            rec = EntityRecord(
                uid=doc._alloc_uid(),
                name=entity.name or "",
                components=list(entity.components()),
            )
            # Every entity the inspector shows has a Transform row; synth one
            # if the file omitted it so the panel has something to edit.
            if rec.find("Transform") is None:
                rec.components.insert(0, at.Transform())
            doc._entities.append(rec)
        doc.path = path
        doc._dirty = False
        doc._undo.clear()
        doc._redo.clear()
        doc.selected_uid = doc._entities[0].uid if doc._entities else None
        return doc

    @classmethod
    def from_ron(cls, text: str) -> "SceneDocument":
        scene = at.Scene.from_ron(text)
        doc = cls()
        for entity in scene.entities():
            rec = EntityRecord(
                uid=doc._alloc_uid(),
                name=entity.name or "",
                components=list(entity.components()),
            )
            if rec.find("Transform") is None:
                rec.components.insert(0, at.Transform())
            doc._entities.append(rec)
        doc._dirty = False
        doc.selected_uid = doc._entities[0].uid if doc._entities else None
        return doc

    # -- undo / redo ---------------------------------------------------------

    def can_undo(self) -> bool:
        return bool(self._undo)

    def can_redo(self) -> bool:
        return bool(self._redo)

    def undo(self) -> bool:
        if not self._undo:
            return False
        self._redo.append(self._snapshot())
        ron = self._undo.pop()
        self._restore(ron)
        self._dirty = True
        return True

    def redo(self) -> bool:
        if not self._redo:
            return False
        self._undo.append(self._snapshot())
        ron = self._redo.pop()
        self._restore(ron)
        self._dirty = True
        return True

    def _restore(self, ron: str) -> None:
        """Replaces the document contents from a RON snapshot, keeping uids
        stable by entity name where possible so selection survives."""
        restored = SceneDocument.from_ron(ron)
        old_uid_by_name = {e.name: e.uid for e in self._entities}
        for rec in restored._entities:
            rec.uid = old_uid_by_name.get(rec.name, rec.uid)
        self._entities = restored._entities
        self._next_uid = max(
            [self._next_uid] + [e.uid + 1 for e in self._entities]
        )
        if self.selected_uid is not None and self.find(self.selected_uid) is None:
            self.selected_uid = self._entities[0].uid if self._entities else None
