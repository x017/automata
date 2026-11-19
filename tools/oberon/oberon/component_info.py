"""Per-component-type metadata for the Oberon inspector.

Each AUTOMATA component the editor can show is described by a small spec:
its display name, and the ordered list of editable fields. A field is a
``(key, label, kind)`` triple where ``key`` is the Python attribute on the
component object and ``kind`` tells the inspector which property widget to
use (``vec3``, ``float``, ``bool``, ``color``, ``rgba``, ``text``).

This module is pure Python and never imports nexa — it is the contract the
inspector (UI) and the document model (edits) both read from, so it stays
unit-testable headless.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Optional, Tuple

# A property widget kind understood by the inspector.
Kind = str

# (attribute key, human label, widget kind)
Field = Tuple[str, str, Kind]


@dataclass(frozen=True)
class ComponentSpec:
    """How Oberon displays and edits one component type."""

    #: The automata class name (e.g. ``"Transform"``).
    type_name: str
    #: Display title in the inspector header.
    title: str
    #: Ordered editable fields.
    fields: Tuple[Field, ...]
    #: True for the single Transform each entity conceptually has — it is
    #: always shown (synthesized when absent) and cannot be removed.
    required: bool = False


# The canonical field specs, keyed by automata class name. Field kinds map
# to nexa inspector property widgets in ``oberon.inspector``.
SPECS: Dict[str, ComponentSpec] = {
    "Transform": ComponentSpec(
        "Transform",
        "Transform",
        (
            ("position", "Position", "vec3"),
            ("rotation_euler", "Rotation (rad)", "vec3"),
            ("scale", "Scale", "vec3"),
        ),
        required=True,
    ),
    "MeshRenderer": ComponentSpec(
        "MeshRenderer",
        "Mesh Renderer",
        (
            ("color", "Color", "rgba"),
        ),
    ),
    "DirectionalLight": ComponentSpec(
        "DirectionalLight",
        "Directional Light",
        (
            ("direction", "Direction", "vec3"),
            ("color", "Color", "color"),
            ("ambient", "Ambient", "float"),
            ("shadows", "Shadows", "bool"),
        ),
    ),
    "Camera3D": ComponentSpec(
        "Camera3D",
        "Camera 3D",
        (
            ("position", "Position", "vec3"),
            ("target", "Target", "vec3"),
            ("fov", "FOV (deg)", "float"),
        ),
    ),
    "Camera2D": ComponentSpec(
        "Camera2D",
        "Camera 2D",
        (
            ("center", "Center", "vec2"),
            ("half_height", "Half Height", "float"),
        ),
    ),
    "RigidBody": ComponentSpec(
        "RigidBody",
        "Rigid Body",
        (
            ("body_type", "Body Type", "text"),
        ),
    ),
    "Collider3D": ComponentSpec(
        "Collider3D",
        "Collider 3D",
        (),
    ),
    "Collider2D": ComponentSpec(
        "Collider2D",
        "Collider 2D",
        (),
    ),
    "CharacterController": ComponentSpec(
        "CharacterController",
        "Character Controller",
        (),
    ),
    "CharacterVelocity": ComponentSpec(
        "CharacterVelocity",
        "Character Velocity",
        (
            ("velocity", "Velocity", "vec3"),
        ),
    ),
}


def spec_for(type_name: str) -> Optional[ComponentSpec]:
    """The spec for a component class name, or ``None`` if unknown."""
    return SPECS.get(type_name)


def type_name_of(component: Any) -> str:
    """The automata class name of a component instance."""
    return type(component).__name__


# --- Factories for "Add Component" -----------------------------------------
# Each factory builds a fresh component of that type with sane defaults.
# They are resolved lazily against the `automata` module so this file stays
# importable without the engine installed.


def _factories() -> Dict[str, Callable[[], Any]]:
    import automata as at  # noqa: PLC0415 - deferred import

    return {
        "MeshRenderer": lambda: at.MeshRenderer.cube(),
        "DirectionalLight": lambda: at.DirectionalLight(),
        "Camera3D": lambda: at.Camera3D(position=(0.0, 2.0, 6.0)),
        "Camera2D": lambda: at.Camera2D(),
        "RigidBody": lambda: at.RigidBody.dynamic(),
        "Collider3D": lambda: at.Collider3D.cuboid(1.0, 1.0, 1.0),
        "Collider2D": lambda: at.Collider2D.cuboid(1.0, 1.0),
        "CharacterVelocity": lambda: at.CharacterVelocity(),
    }


#: Types the "Add Component" menu offers, in display order.
ADDABLE: Tuple[str, ...] = (
    "MeshRenderer",
    "DirectionalLight",
    "Camera3D",
    "Camera2D",
    "RigidBody",
    "Collider3D",
    "Collider2D",
    "CharacterVelocity",
)


def make_component(type_name: str) -> Any:
    """A fresh component of ``type_name`` with defaults.

    Raises ``KeyError`` for a type that has no factory.
    """
    factory = _factories().get(type_name)
    if factory is None:
        raise KeyError(f"no factory for component type {type_name!r}")
    return factory()
