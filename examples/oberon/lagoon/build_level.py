#!/usr/bin/env python3
"""Writes the Lagoon scene: a small island in a turquoise sea.

    python examples/oberon/lagoon/build_level.py   # writes level.ron
    cargo run -p oberon --release examples/oberon/lagoon/level.ron

The scene shows off Water. The sea floor rises in steps toward the island,
so the colour shifts from deep blue to turquoise as the water gets
shallower. A sandy ramp runs into the sea on two sides, and that's where
the shoreline foam forms. There's also a clear freshwater pool on the
island, a pier, rocks breaking the surface, and crates, buoys and a boat
riding the swell: real rigid bodies, floated by the water's buoyancy (each
sits as deep as its collider density says) and rocked by its waves. Pick
Beautiful in Oberon's Graphics tab
to see refraction, depth colour and foam. The other presets draw the same
water blended over the scene.

Everything is plain cubes plus two small models (`models/`), so you can
rebuild or rearrange it by hand in Oberon just as well.
"""

import math
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

SAND = (0.74, 0.63, 0.44, 1.0)
WET_SAND = (0.56, 0.48, 0.33, 1.0)
REEF = (0.46, 0.42, 0.31, 1.0)
SEABED = (0.32, 0.3, 0.24, 1.0)
MOSS = (0.36, 0.42, 0.28, 1.0)
GRASS = (0.3, 0.5, 0.2, 1.0)
ROCK = (0.34, 0.34, 0.36, 1.0)
WOOD = (0.4, 0.27, 0.16, 1.0)
HULL = (0.62, 0.14, 0.09, 1.0)
BUOY = (0.95, 0.22, 0.04, 1.0)

# The sea: turquoise in the shallows, deep blue further out.
OCEAN = dict(
    size=(240.0, 240.0),
    deep_color=(0.004, 0.045, 0.09),
    shallow_color=(0.06, 0.5, 0.52),
    clarity=2.5,
    # A long swell: the 128-cell grid over 240 m draws waves from ~8 m up
    # (shorter ones only ripple the shading), and floating things ride it.
    wave_height=0.35,
    wave_length=20.0,
    wave_speed=1.0,
    wave_direction=25.0,
    foam=0.8,
    # Past its 240 m the sea carries on, calm, to the horizon.
    endless=True,
)
# The freshwater pool: calm and clear enough to see the bottom.
POOL = dict(
    size=(6.0, 6.0),
    deep_color=(0.01, 0.08, 0.07),
    shallow_color=(0.12, 0.45, 0.35),
    clarity=3.0,
    wave_height=0.02,
    wave_length=1.6,
    wave_speed=0.6,
    foam=0.2,
)
POOL_CENTER = (-4.0, 0.82, -1.0)

# Solid blocks: name, centre, size, rotation (radians), colour.
# The dry beach (top at y = 1) is four slabs around the pool's hole.
BLOCKS = [
    ("Seabed", (0.0, -7.5, 0.0), (240.0, 1.0, 240.0), (0.0, 0.0, 0.0), SEABED),
    ("Reef", (2.0, -2.6, 1.0), (46.0, 1.0, 40.0), (0.0, 0.17, 0.0), REEF),
    ("Shallows", (0.0, -1.2, 0.5), (30.0, 1.0, 25.0), (0.0, -0.12, 0.0), WET_SAND),
    ("Sandbar West", (-13.0, -1.35, 9.0), (14.0, 1.0, 10.0), (0.0, 0.6, 0.0), WET_SAND),
    ("Sandbar East", (14.0, -1.35, -8.0), (12.0, 1.0, 12.0), (0.0, -0.5, 0.0), WET_SAND),
    # Ramps slope ~8° from the beach down under the sea: the shoreline.
    ("South Beach", (2.0, -0.35, 11.0), (20.0, 0.6, 9.0), (0.15, 0.0, 0.0), SAND),
    ("East Beach", (13.0, -0.35, 0.0), (9.0, 0.6, 14.0), (0.0, 0.0, -0.15), SAND),
    ("Beach West", (-8.5, -0.2, 0.0), (3.0, 2.4, 15.0), (0.0, 0.0, 0.0), SAND),
    ("Beach East", (4.5, -0.2, 0.0), (11.0, 2.4, 15.0), (0.0, 0.0, 0.0), SAND),
    ("Beach North", (-4.0, -0.2, -5.75), (6.0, 2.4, 3.5), (0.0, 0.0, 0.0), SAND),
    ("Beach South", (-4.0, -0.2, 4.75), (6.0, 2.4, 5.5), (0.0, 0.0, 0.0), SAND),
    ("Pool Floor", (-4.0, -0.3, -1.0), (6.0, 1.2, 6.0), (0.0, 0.0, 0.0), MOSS),
    ("Hill", (5.0, 1.2, -2.0), (7.0, 0.5, 6.0), (0.0, 0.3, 0.0), GRASS),
    ("Hilltop", (5.5, 1.6, -2.5), (4.0, 0.6, 3.5), (0.0, 0.5, 0.0), GRASS),
]

# Rocks breaking the surface: centre, size, rotation.
ROCKS = [
    ((-13.0, 0.0, -6.0), (2.2, 2.0, 1.8), (0.3, 0.6, 0.2)),
    ((-12.5, -0.2, 6.5), (1.6, 1.4, 1.5), (0.1, 1.1, 0.4)),
    ((9.0, 0.1, -11.0), (2.6, 2.2, 2.0), (0.2, 0.3, 0.1)),
    ((-4.0, -0.3, -12.0), (1.4, 1.2, 1.3), (0.5, 0.2, 0.3)),
    ((17.5, -0.4, 9.0), (1.8, 1.5, 1.6), (0.2, 0.9, 0.5)),
    ((-18.0, -0.6, 14.0), (2.5, 2.0, 2.2), (0.4, 0.4, 0.1)),
]

# Trees on the beach: position, yaw (radians), scale.
TREES = [
    ((7.0, 1.0, 3.5), 0.3, 1.3),
    ((8.5, 1.0, -5.5), 1.2, 1.1),
    ((-8.5, 1.0, -5.0), 2.0, 1.2),
    ((1.0, 1.0, -6.0), 0.7, 0.9),
    ((-1.5, 1.0, 5.5), 2.6, 1.0),
    ((2.5, 1.6, -2.0), 1.6, 0.8),
]

PIER_X = -8.5
PIER_START, PIER_END = 6.0, 22.0

# Floating things: name, position, rotation, size, model or colour, and
# collider density. Water is density 2, so 1.0 floats half under, 0.5 a
# quarter under; above 2 it sinks. Crate 1 starts in the air: it drops in
# when Play starts, and rings spread from the splash.
FLOATING = [
    ("Crate 1", (6.0, 4.0, 19.0), (0.1, 0.5, 0.0), (0.8, 0.8, 0.8), "models/crate.obj", 1.0),
    ("Crate 2", (8.2, -0.28, 22.5), (0.0, 1.3, 0.15), (0.7, 0.7, 0.7), "models/crate.obj", 1.0),
    ("Buoy 1", (-2.5, 0.1, 21.5), (0.0, 0.0, 0.0), (0.5, 0.7, 0.5), BUOY, 0.8),
    ("Buoy 2", (13.0, 0.1, 16.0), (0.0, 0.0, 0.0), (0.5, 0.7, 0.5), BUOY, 0.8),
    ("Buoy 3", (-19.5, 0.1, 24.0), (0.0, 0.0, 0.0), (0.5, 0.7, 0.5), BUOY, 0.8),
]

# The boat cruises an ellipse round the island (scripts/motor_boat.py),
# clear of the pier, the rocks and the buoys, and leaves a wake.
BOAT_LOOP = {
    "center": (0.0, 0.0, 2.0),
    "radius_x": 24.0,
    "radius_z": 25.0,
    "speed": 4.5,
    "draft": 0.18,
}
BOAT_SIZE = (1.6, 0.6, 3.8)
BOAT_START_ANGLE = -0.3  # radians round the loop (0 = due +Z of its centre)

CAMERA = {
    "target": (0.0, 0.5, 4.0),
    "distance": 32.0,
    "yaw": 0.35,
    "pitch": 0.33,
    "spin": 0.05,
    # After Tab, the camera follows the swimmer.
    "follow": "Swimmer",
    "follow_distance": 6.0,
}

# The swimmer waits at the end of the pier (scripts/swimmer.py): Tab takes
# control, and it's a short run and a jump into the sea.
SWIMMER_SIZE = (0.5, 1.7, 0.5)
SWIMMER_AT = (PIER_X, 2.13, 20.5)
SWIMMER = (0.95, 0.75, 0.1, 1.0)


def sun_direction(azimuth, elevation):
    """The direction light travels, from the Graphics tab's angles (degrees)."""
    az, el = math.radians(azimuth), math.radians(elevation)
    return (-math.cos(el) * math.sin(az), -math.sin(el), -math.cos(el) * math.cos(az))


def orbit_eye(target, distance, yaw, pitch):
    """Where scripts/orbit_camera.py puts the camera (yaw 0 = from +Z)."""
    flat = distance * math.cos(pitch)
    return (
        target[0] + flat * math.sin(yaw),
        target[1] + distance * math.sin(pitch),
        target[2] + flat * math.cos(yaw),
    )


def block(name, center, size, rotation, color):
    """A solid cube: visible and walkable (box collider, no rigid body)."""
    return (
        at.Entity(name)
        .add(at.Transform(position=center, rotation_euler=rotation, scale=size))
        .add(at.MeshRenderer.cube(size=1.0, color=color))
        .add(at.Collider3D.cuboid(*size))  # full size: physics ignores scale
    )


def build():
    at.set_asset_root(HERE)  # model paths are relative to this folder
    scene = at.Scene()
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(
                direction=sun_direction(150, 32),
                color=(1.0, 0.95, 0.85),
                ambient=0.45,
            )
        )
    )
    scene.add(
        at.Entity("Environment").add(
            at.Environment(
                sky_color=(0.75, 0.85, 1.0),
                ground_color=(0.55, 0.5, 0.42),
                background=(0.62, 0.78, 0.95),
                exposure=1.0,
                tonemapping="aces",
                fog=True,
                fog_color=(0.62, 0.78, 0.95),
                fog_density=0.006,
                fog_start=35.0,
                # A sea haze lying on the water, lit up toward the sun.
                fog_height=1.0,
                fog_falloff=0.25,
                fog_sun=0.8,
                sky=True,
                zenith_color=(0.16, 0.34, 0.78),
                clouds=0.35,
                # Beautiful only: sun shafts, and a soft blur away from the
                # camera's target.
                god_rays=0.6,
                depth_of_field=0.5,
                focus_range=6.0,
                post="film",
            )
        )
    )
    scene.add(
        at.Entity("Sea")
        .add(at.Transform(position=(0.0, 0.0, 0.0)))
        .add(at.Water(**OCEAN))
    )
    scene.add(at.Entity("Pool").add(at.Transform(position=POOL_CENTER)).add(at.Water(**POOL)))

    for name, center, size, rotation, color in BLOCKS:
        scene.add(block(name, center, size, rotation, color))
    for i, (center, size, rotation) in enumerate(ROCKS, start=1):
        scene.add(block(f"Rock {i}", center, size, rotation, ROCK))
    for i, (position, yaw, scale) in enumerate(TREES, start=1):
        scene.add(
            at.Entity(f"Tree {i}")
            .add(at.Transform(position=position, rotation_euler=(0.0, yaw, 0.0),
                              scale=(scale, scale, scale)))
            .add(at.MeshRenderer.model("models/tree.glb"))
        )

    # The pier: a deck on two rows of posts reaching down to the seabed.
    length = PIER_END - PIER_START
    scene.add(block("Pier", (PIER_X, 1.15, PIER_START + length / 2), (2.2, 0.15, length),
                    (0.0, 0.0, 0.0), WOOD))
    for i, z in enumerate(range(int(PIER_START) + 1, int(PIER_END) + 1, 3), start=1):
        for side, dx in (("L", -0.9), ("R", 0.9)):
            scene.add(block(f"Post {i}{side}", (PIER_X + dx, -0.55, float(z)),
                            (0.25, 3.3, 0.25), (0.0, 0.0, 0.0), WOOD))

    for name, position, rotation, size, look, density in FLOATING:
        if isinstance(look, str):
            # The model's own shape (scaled with the transform).
            visual = at.MeshRenderer.model(look)
            collider = at.Collider3D.convex_hull(look)
        else:
            # A unit cube scaled to `size`; box colliders take world sizes.
            # Buoys are glossy painted plastic; the hull is plain wood.
            gloss = 0.25 if name.startswith("Buoy") else None
            visual = at.MeshRenderer.cube(size=1.0, color=look, roughness=gloss)
            collider = at.Collider3D.cuboid(*size)
        scene.add(
            at.Entity(name)
            .add(at.Transform(position=position, rotation_euler=rotation, scale=size))
            .add(visual)
            .add(at.RigidBody.dynamic())
            .add(collider.density(density))
        )

    # The swimmer: a character its script walks, jumps and swims.
    swimmer = (
        at.Entity("Swimmer")
        .add(at.Transform(position=SWIMMER_AT, scale=SWIMMER_SIZE))
        .add(at.MeshRenderer.cube(size=1.0, color=SWIMMER))
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider3D.cuboid(*SWIMMER_SIZE))
        .add(at.CharacterController(snap_to_ground=0.2))
        .add(at.CharacterVelocity())
    )
    swimmer.script = "scripts/swimmer.py"
    scene.add(swimmer)

    # The boat: a kinematic body the script steers (the water doesn't push
    # it, but it pushes the water, and anything floating in its way).
    cx, _, cz = BOAT_LOOP["center"]
    rx, rz = BOAT_LOOP["radius_x"], BOAT_LOOP["radius_z"]
    a = BOAT_START_ANGLE
    boat_at = (cx + rx * math.sin(a), 0.15, cz + rz * math.cos(a))
    boat_yaw = math.atan2(rx * math.cos(a), -rz * math.sin(a))
    boat = (
        at.Entity("Boat")
        .add(at.Transform(position=boat_at, rotation_euler=(0.0, boat_yaw, 0.0), scale=BOAT_SIZE))
        .add(at.MeshRenderer.cube(size=1.0, color=HULL))
        .add(at.RigidBody.kinematic_position())
        .add(at.Collider3D.cuboid(*BOAT_SIZE))
    )
    boat.script = "scripts/motor_boat.py"
    for key, value in BOAT_LOOP.items():
        boat.attributes[key] = value
    scene.add(boat)

    # Start where orbit_camera.py will place it, so the editor shows the
    # same view as the first frame of Play.
    start = orbit_eye(CAMERA["target"], CAMERA["distance"], CAMERA["yaw"], CAMERA["pitch"])
    camera = (
        at.Entity("Main Camera")
        .add(at.Transform(position=start))
        .add(at.Camera3D(position=start, target=CAMERA["target"], fov=55.0))
    )
    camera.script = "scripts/orbit_camera.py"
    for key, value in CAMERA.items():
        camera.attributes[key] = value
    scene.add(camera)
    # Dust in the sunlight: a box of motes that travels with the camera.
    scene.add(
        at.Entity("Motes")
        .add(at.Transform(position=(0.0, 2.0, 0.0)))
        .add(at.ParticleEmitter.preset("motes"))
    )
    return scene


if __name__ == "__main__":
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
