"""AI helpers for scripts: state machines, behaviour trees, and senses.

The same classes scripts reach through ``ctx.ai`` (which also has the
senses: ``can_see``, ``visible``, ``noise`` and ``hear``)::

    from automata import ai

    brain = ai.Selector(
        ai.Sequence(ai.Condition(sees_player), shoot),
        patrol,
    )

See the manual's "AI" page.
"""

from .script_api import (
    AI,
    FAILURE,
    RUNNING,
    SUCCESS,
    Action,
    Condition,
    Cooldown,
    Invert,
    Node,
    Noise,
    Parallel,
    Repeat,
    Selector,
    Sequence,
    StateMachine,
    Succeed,
    Wait,
)

__all__ = [
    "AI",
    "FAILURE",
    "RUNNING",
    "SUCCESS",
    "Action",
    "Condition",
    "Cooldown",
    "Invert",
    "Node",
    "Noise",
    "Parallel",
    "Repeat",
    "Selector",
    "Sequence",
    "StateMachine",
    "Succeed",
    "Wait",
]
