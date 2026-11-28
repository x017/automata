"""jogger: runs up the track, stops dead, turns round and runs back.

The ponytail and skirt aren't animated: they are spring bones (see
``models/runner.glb.import.ron``), so they stream out behind while she
runs and swing on forwards when she stops, then settle.
"""

import math

LAP = 9.0  # metres from one end of the track to the other
RUN = 4.2  # m/s; the Run clip's stride matches this
PAUSE = 2.5  # seconds standing at each end


def next_leg(state, dt):
    """Advances the jog by ``dt`` seconds. ``state`` holds ``leg`` ("run"
    or "rest"), ``left`` (metres or seconds left of it) and ``heading``
    (+1 or -1 along the track). Returns how far to move this frame."""
    if state["leg"] == "run":
        step = min(RUN * dt, state["left"])
        state["left"] -= step
        if state["left"] <= 1e-6:
            state.update(leg="rest", left=PAUSE)
        return step * state["heading"]
    state["left"] -= dt
    if state["left"] <= 0.0:
        state.update(leg="run", left=LAP, heading=-state["heading"])
    return 0.0


def start(entity, ctx):
    entity.state.update(leg="rest", left=1.0, heading=-1.0)
    entity.state["x0"] = entity.transform.position[0]


def update(entity, ctx):
    state = entity.state
    dx = next_leg(state, ctx.time.delta)
    x, y, z = entity.transform.position
    entity.transform.position = (x + dx, y, z)
    # The model faces +Z; turn it to face along the track.
    entity.transform.rotation_euler = (0.0, math.pi / 2 * state["heading"], 0.0)
    if state["leg"] == "run":
        entity.animation.play("Run", fade=0.15)
    else:
        entity.animation.play("Idle", fade=0.2)
