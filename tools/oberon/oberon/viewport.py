"""A tiny dependency-free software 3D wireframe renderer for the viewport.

The engine's real renderer is GPU (wgpu); embedding its frames into a nexa
texture is the engine-side M1/M2 hook (see docs/design/oberon.md). Until
that lands, Oberon draws the scene itself: a perspective projection of the
document's meshes, lights, and cameras as wireframe lines onto an RGBA
buffer, which nexa displays via ``ui.image``.

Pure Python, no nexa and no engine imports — fully unit-testable headless.
"""

from __future__ import annotations

import math
from typing import List, Optional, Sequence, Tuple

Vec3 = Tuple[float, float, float]
Color = Tuple[int, int, int, int]

# Axis / editor-chrome colors (dark-white sci-fi: bright lines on near-black).
COLOR_GRID = (56, 60, 72, 255)
COLOR_AXIS_X = (222, 92, 96, 255)
COLOR_AXIS_Z = (96, 148, 222, 255)
COLOR_MESH = (232, 234, 240, 255)
COLOR_SELECTED = (120, 200, 255, 255)
COLOR_LIGHT = (245, 205, 110, 255)
COLOR_CAMERA = (140, 220, 180, 255)


def _sub(a: Vec3, b: Vec3) -> Vec3:
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _cross(a: Vec3, b: Vec3) -> Vec3:
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _dot(a: Vec3, b: Vec3) -> float:
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def _norm(a: Vec3) -> Vec3:
    length = math.sqrt(_dot(a, a)) or 1.0
    return (a[0] / length, a[1] / length, a[2] / length)


class Camera:
    """A simple orbitable perspective camera."""

    def __init__(self) -> None:
        self.target: Vec3 = (0.0, 0.0, 0.0)
        self.distance = 8.0
        self.yaw = math.radians(35.0)
        self.pitch = math.radians(28.0)
        self.fov_y = math.radians(55.0)

    def eye(self) -> Vec3:
        cp = math.cos(self.pitch)
        return (
            self.target[0] + self.distance * cp * math.sin(self.yaw),
            self.target[1] + self.distance * math.sin(self.pitch),
            self.target[2] + self.distance * cp * math.cos(self.yaw),
        )

    def basis(self) -> Tuple[Vec3, Vec3, Vec3]:
        """(right, up, forward-from-eye-to-target)."""
        eye = self.eye()
        fwd = _norm(_sub(self.target, eye))
        right = _norm(_cross(fwd, (0.0, 1.0, 0.0)))
        up = _cross(right, fwd)
        return right, up, fwd

    def orbit(self, dx: float, dy: float) -> None:
        self.yaw += dx * 0.008
        self.pitch = max(-1.5, min(1.5, self.pitch + dy * 0.008))

    def zoom(self, delta: float) -> None:
        self.distance = max(1.0, min(200.0, self.distance * (1.0 - delta * 0.001)))

    def pan(self, dx: float, dy: float) -> None:
        right, up, _ = self.basis()
        k = self.distance * 0.0016
        self.target = (
            self.target[0] - right[0] * dx * k + up[0] * dy * k,
            self.target[1] - right[1] * dx * k + up[1] * dy * k,
            self.target[2] - right[2] * dx * k + up[2] * dy * k,
        )


class FrameBuffer:
    """An RGBA8 pixel buffer with a Bresenham line drawer."""

    def __init__(self, width: int, height: int, clear: Color = (18, 19, 24, 255)) -> None:
        self.width = max(1, int(width))
        self.height = max(1, int(height))
        self.clear = clear
        self.pixels = bytearray(self.width * self.height * 4)
        self.fill(clear)

    def fill(self, color: Color) -> None:
        r, g, b, a = color
        row = bytes((r, g, b, a)) * self.width
        for y in range(self.height):
            start = y * self.width * 4
            self.pixels[start : start + self.width * 4] = row

    def set(self, x: int, y: int, color: Color) -> None:
        if 0 <= x < self.width and 0 <= y < self.height:
            i = (y * self.width + x) * 4
            self.pixels[i : i + 4] = bytes(color)

    def line(self, p0: Tuple[float, float], p1: Tuple[float, float], color: Color) -> None:
        x0, y0 = int(round(p0[0])), int(round(p0[1]))
        x1, y1 = int(round(p1[0])), int(round(p1[1]))
        dx = abs(x1 - x0)
        dy = -abs(y1 - y0)
        sx = 1 if x0 < x1 else -1
        sy = 1 if y0 < y1 else -1
        err = dx + dy
        while True:
            self.set(x0, y0, color)
            if x0 == x1 and y0 == y1:
                break
            e2 = 2 * err
            if e2 >= dy:
                err += dy
                x0 += sx
            if e2 <= dx:
                err += dx
                y0 += sy

    def to_bytes(self) -> bytes:
        return bytes(self.pixels)


def _cube_edges(size: float) -> List[Tuple[Vec3, Vec3]]:
    h = size / 2.0
    c = [
        (-h, -h, -h), (h, -h, -h), (h, h, -h), (-h, h, -h),
        (-h, -h, h), (h, -h, h), (h, h, h), (-h, h, h),
    ]
    idx = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ]
    return [(c[a], c[b]) for a, b in idx]


def _quad_edges(size: float) -> List[Tuple[Vec3, Vec3]]:
    h = size / 2.0
    c = [(-h, -h, 0.0), (h, -h, 0.0), (h, h, 0.0), (-h, h, 0.0)]
    idx = [(0, 1), (1, 2), (2, 3), (3, 0)]
    return [(c[a], c[b]) for a, b in idx]


class ViewportRenderer:
    """Renders a scene of ``(position, scale, edges, color)`` into a buffer."""

    def __init__(self, camera: Optional[Camera] = None) -> None:
        self.camera = camera or Camera()

    def project(self, point: Vec3, width: int, height: int) -> Optional[Tuple[float, float]]:
        right, up, fwd = self.camera.basis()
        eye = self.camera.eye()
        rel = _sub(point, eye)
        z = _dot(rel, fwd)
        if z <= 0.05:
            return None
        x = _dot(rel, right)
        y = _dot(rel, up)
        f = (height / 2.0) / math.tan(self.camera.fov_y / 2.0)
        sx = width / 2.0 + (x * f) / z
        sy = height / 2.0 - (y * f) / z
        return (sx, sy)

    def draw_edges(
        self,
        fb: FrameBuffer,
        edges: Sequence[Tuple[Vec3, Vec3]],
        origin: Vec3,
        scale: Vec3,
        color: Color,
    ) -> None:
        for a, b in edges:
            wa = (origin[0] + a[0] * scale[0], origin[1] + a[1] * scale[1], origin[2] + a[2] * scale[2])
            wb = (origin[0] + b[0] * scale[0], origin[1] + b[1] * scale[1], origin[2] + b[2] * scale[2])
            pa = self.project(wa, fb.width, fb.height)
            pb = self.project(wb, fb.width, fb.height)
            if pa is not None and pb is not None:
                fb.line(pa, pb, color)

    def draw_grid(self, fb: FrameBuffer, half: int = 10, step: float = 1.0) -> None:
        for i in range(-half, half + 1):
            color_a = COLOR_AXIS_X if i == 0 else COLOR_GRID
            color_b = COLOR_AXIS_Z if i == 0 else COLOR_GRID
            self.draw_edges(
                fb, [((i * step, 0.0, -half * step), (i * step, 0.0, half * step))],
                (0.0, 0.0, 0.0), (1.0, 1.0, 1.0), color_a,
            )
            self.draw_edges(
                fb, [((-half * step, 0.0, i * step), (half * step, 0.0, i * step))],
                (0.0, 0.0, 0.0), (1.0, 1.0, 1.0), color_b,
            )


def render_document(
    doc,
    camera: Camera,
    width: int,
    height: int,
    selected_uid: Optional[int] = None,
) -> FrameBuffer:
    """Renders the document's entities as wireframes into a fresh buffer."""
    fb = FrameBuffer(width, height)
    renderer = ViewportRenderer(camera)
    renderer.draw_grid(fb)

    cube = _cube_edges(1.0)
    quad = _quad_edges(1.0)

    for rec in doc.entities:
        transform = rec.find("Transform")
        pos = getattr(transform, "position", (0.0, 0.0, 0.0)) if transform else (0.0, 0.0, 0.0)
        scale = getattr(transform, "scale", (1.0, 1.0, 1.0)) if transform else (1.0, 1.0, 1.0)
        selected = rec.uid == selected_uid
        base = COLOR_SELECTED if selected else COLOR_MESH

        mesh = rec.find("MeshRenderer")
        if mesh is not None:
            edges = cube if getattr(mesh, "kind", "cube") == "cube" else quad
            renderer.draw_edges(fb, edges, pos, scale, base)
        elif transform is not None:
            # No mesh: draw a small marker cross so the entity is visible.
            m = 0.25
            marker = [
                ((-m, 0, 0), (m, 0, 0)),
                ((0, -m, 0), (0, m, 0)),
                ((0, 0, -m), (0, 0, m)),
            ]
            renderer.draw_edges(fb, marker, pos, (1.0, 1.0, 1.0), base)

        if rec.find("DirectionalLight") is not None:
            sun = [((0, 0, 0), (0, 1.2, 0)), ((-0.4, 0.9, 0), (0.4, 0.9, 0)),
                   ((0, 0.9, -0.4), (0, 0.9, 0.4))]
            renderer.draw_edges(fb, sun, pos, (1.0, 1.0, 1.0), COLOR_LIGHT)
        if rec.find("Camera3D") is not None or rec.find("Camera2D") is not None:
            frustum = [((0, 0, 0), (-0.4, 0.3, -0.8)), ((0, 0, 0), (0.4, 0.3, -0.8)),
                       ((0, 0, 0), (0.4, -0.3, -0.8)), ((0, 0, 0), (-0.4, -0.3, -0.8)),
                       ((-0.4, 0.3, -0.8), (0.4, 0.3, -0.8)), ((0.4, 0.3, -0.8), (0.4, -0.3, -0.8)),
                       ((0.4, -0.3, -0.8), (-0.4, -0.3, -0.8)), ((-0.4, -0.3, -0.8), (-0.4, 0.3, -0.8))]
            renderer.draw_edges(fb, frustum, pos, (1.0, 1.0, 1.0), COLOR_CAMERA)

    return fb
