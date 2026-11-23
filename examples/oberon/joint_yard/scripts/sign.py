"""The sign on the post: welded on by a Fixed joint with a break force.

When the wrecking ball hits it hard enough the joint snaps, and the engine
calls ``on_joint_broken`` (and publishes ``Sign:joint:broken``).
"""


def on_joint_broken(entity, ctx):
    entity.attributes["snapped"] = True
    entity.attributes["force"] = round(entity.joint.force)
    print(f"Crash! The sign snapped off its post ({entity.joint.force:.0f} N).")
