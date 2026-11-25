# Animation graphs

An animation graph decides which clips a character plays, so your scripts
don't have to. The script says *what's going on*: how fast the character
moves, whether it's on the ground, that the player pressed jump. The graph
turns that into clips: it blends Idle into Walk into Run as the speed
rises, cuts to Jump when the jump trigger fires, and goes back to running
once it lands.

Graphs are `.anim.ron` files in your project (put them in `anims/`). An
[Animation](models.md#animation) component uses one through its **graph**
field. Then its **Clip**, **Loop** and **Play On Start** stop mattering:
the graph picks the clips, and **Speed** scales the whole graph.

```ron
// level.ron
animation: (graph: "anims/robot.anim.ron"),
```

## What's in a graph

| Part | What it is |
|---|---|
| **Parameters** | Named values scripts set: `Float` (a speed), `Int` (a weapon slot), `Bool` (grounded) and `Trigger` (jump: on until a transition uses it) |
| **Layers** | State machines stacked bottom to top. The first one animates the whole model. A later one animates only the bones in its **mask** (say, the upper body), over the layers below, at its **weight** |
| **States** | What a layer can be doing. Each plays a **motion**: one clip, a 1D or 2D **blend space**, or nothing (`Empty`) |
| **Transitions** | The ways from one state to the next. Each has **conditions**, a **fade** time and, optionally, an **exit time** |

Here's a robot that walks, runs, jumps and waves with its arms while it
walks:

```ron
(
    parameters: [
        (name: "speed", kind: Float(0.0)),
        (name: "grounded", kind: Bool(true)),
        (name: "jump", kind: Trigger),
        (name: "wave", kind: Trigger),
    ],
    layers: [
        (
            name: "Base",
            states: [
                (
                    name: "Move",
                    motion: Blend1D(param: "speed", points: [
                        (clip: "Idle", at: 0.0),
                        (clip: "Walking", at: 2.2),
                        (clip: "Running", at: 5.0),
                    ]),
                    position: (0.0, 0.0),
                ),
                (name: "Jump", motion: Clip("Jump"), looping: false, position: (240.0, 0.0)),
            ],
            transitions: [
                (from: "*", to: "Jump", when: [Triggered("jump")], fade: 0.1),
                (from: "Jump", to: "Move", when: [IsTrue("grounded")], fade: 0.25, exit_time: 0.6),
            ],
        ),
        (
            name: "Arms",
            mask: ["Torso"],  // the chest, arms and head
            states: [
                (name: "Off", motion: Empty, position: (0.0, 0.0)),
                (name: "Wave", motion: Clip("Wave"), looping: false, position: (240.0, 0.0)),
            ],
            transitions: [
                (from: "Off", to: "Wave", when: [Triggered("wave")], fade: 0.2),
                (from: "Wave", to: "Off", fade: 0.3),
            ],
        ),
    ],
)
```

Fields you leave out take their defaults, so hand-written graphs stay
short. `position` is where the editor draws the state.

## States and motions

| Field | Default | Meaning |
|---|---|---|
| `name` | | Unique in its layer. Scripts and transitions use it |
| `motion` | `Empty` | `Clip("Walking")`, `Blend1D(...)`, `Blend2D(...)` or `Empty` |
| `speed` | 1 | Playback rate |
| `speed_param` | none | A number parameter the rate is multiplied by (play the walk faster as the character speeds up) |
| `looping` | true | Off plays the motion once and holds the last frame; the clip's `end` event fires |

A layer starts in its `entry` state, or in its first state when `entry` is
left out.

### Blend spaces

A **1D blend space** spreads clips along one parameter. The two clips on
either side of the value blend: with Walking at 2.2 and Running at 5, a
speed of 3.6 is half of each. Below the first point or past the last, that
clip plays alone.

A **2D blend space** spreads clips over two parameters, for strafing:

```ron
motion: Blend2D(x: "side", y: "forward", points: [
    (clip: "Idle", at: (0.0, 0.0)),
    (clip: "WalkForward", at: (0.0, 1.0)),
    (clip: "WalkBack", at: (0.0, -1.0)),
    (clip: "StrafeLeft", at: (-1.0, 0.0)),
    (clip: "StrafeRight", at: (1.0, 0.0)),
]),
```

Every clip weighs in by how close the point is (gradient band
interpolation, the "freeform" kind). On a clip's point, that clip plays
alone.

Either way the clips stay **in step**: they share one position through
their cycle and move at the average of their lengths, weighted by how much
each shows. A walk and a run put the same foot down together, so blending
them never makes the feet skate. Events come from the clip that shows most,
and so does root motion, blended by weight.

## Transitions

| Field | Default | Meaning |
|---|---|---|
| `from` | | The state it leaves, or `"*"` for **any state** in the layer |
| `to` | | The state it goes to |
| `when` | none | Conditions that must all hold |
| `fade` | 0.2 | Seconds to crossfade |
| `exit_time` | none | How far through its motion the state must have played first: 0.5 is halfway, 1 is once through, 2 twice |

Conditions:

| Condition | Holds when |
|---|---|
| `Above("speed", 0.1)`, `Below("speed", 0.1)` | The number is greater or less |
| `Equals("slot", 2)`, `NotEquals("slot", 2)` | The number is (or isn't) that whole number |
| `IsTrue("grounded")`, `IsFalse("grounded")` | The bool is on or off |
| `Triggered("jump")` | The trigger is set. Firing the transition clears it |

Each frame, every layer checks its transitions before it plays on.
Any-state transitions go first, then the current state's own, in file
order. The first one that's ready fires. A transition with no conditions
and no exit time fires when the state's motion ends, which is how a jump
or a wave hands back to the state before. An any-state transition needs a
condition, or it would fire all the time.

A transition can fire in the middle of a fade. The new fade then starts
from whatever the layer showed at that moment, so nothing jumps.

## Layers and masks

A layer's `mask` lists bones by name. The layer animates each one and
everything below it in the skeleton, so `["Torso"]` covers the chest,
arms and head, and the legs stay with the layers underneath. With no mask,
the layer covers the whole model.

`weight` (0 to 1, default 1) sets how strongly the layer shows over the
ones below. An `Empty` state switches the layer off, and fading to or from
one fades the layer out or in. That makes "Off" plus a one-shot clip the
usual shape for an upper-body layer: wave, salute, reload.

The first layer always shows fully and ignores its weight.

## When something's wrong

A graph with mistakes still runs: transitions to states that don't exist,
conditions on unknown parameters, and clips the model doesn't have are
left out, with a warning in the log naming each one. A graph file that
doesn't load at all (missing, or not valid RON) is logged too, and the
Animation plays its **Clip** instead.
