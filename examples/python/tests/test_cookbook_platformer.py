"""The Cookbook recipes, tested inside the capstone game (headless).

    pytest examples/python/tests
"""

import pytest

import cookbook_platformer as cb


class FakeControls:
    """An action map whose jump is "pressed" on chosen frames."""

    def __init__(self, jump_frames):
        self.jump_frames = set(jump_frames)
        self.frame = 0

    def just_pressed(self, action, _input):
        return action == "jump" and self.frame in self.jump_frames

    def axis(self, _action, _input):
        return 0.0


def play(frames, jump_frames=(), probe=None, fps=None):
    """Runs the cookbook game headless; `probe(game, ctx)` runs after each frame.

    Headless time is real time, so tests that need gravity to act between
    frames pass `fps=60`.
    """

    class Probe(cb.CookbookPlatformer):
        headless = True
        target_fps = fps

        def on_start(self):
            super().on_start()
            self.controls = FakeControls(jump_frames)
            self.log = []

        def on_update(self, ctx):
            self.controls.frame = ctx.time.frame
            super().on_update(ctx)
            if probe:
                probe(self, ctx)
            if ctx.time.frame >= frames:
                ctx.quit()

    game = Probe()
    game.run()
    return game


def test_it_plays_and_the_player_lands():
    def probe(game, ctx):
        game.y = ctx.scene["player"].transform.position[1]

    game = play(120, probe=probe)
    assert game.y == pytest.approx(1.4, abs=0.1)
    assert not game.paused


def test_double_jump_but_never_a_third():
    def probe(game, ctx):
        game.log.append((ctx.time.frame, game.vy, game.jumps_left))

    # Land, jump from the ground, jump again in the air, then try a third.
    game = play(80, jump_frames=(40, 46, 52), probe=probe, fps=60)
    by_frame = {f: (vy, left) for f, vy, left in game.log}
    assert by_frame[40] == (pytest.approx(cb.JUMP_SPEED), 1), "ground jump"
    assert by_frame[46] == (pytest.approx(cb.JUMP_SPEED), 0), "air jump"
    assert by_frame[52][0] < cb.JUMP_SPEED - 0.5, "no third jump"


def test_a_hit_shakes_the_camera():
    def probe(game, ctx):
        if ctx.time.frame == 2:
            ctx.events.send("hit", {"speed": 20.0})
        if ctx.time.frame == 4:  # handlers run after on_update, so look a frame later
            game.shake_seen = game.shake

    game = play(5, probe=probe)
    assert 0.0 < game.shake_seen <= cb.SHAKE_TIME


def test_waves_spawn_crates():
    counts = {}

    def probe(game, ctx):
        if ctx.time.frame == 2:
            counts["before"] = ctx.scene.entity_count
            game.next_wave = 0.0  # the timer runs out next frame
        if ctx.time.frame == 5:
            counts["after"] = ctx.scene.entity_count

    game = play(5, probe=probe)
    assert game.wave == 1
    assert counts["after"] == counts["before"] + 1


def test_finishing_a_level_loads_the_next():
    def probe(game, ctx):
        if ctx.time.frame == 2:
            game.coins.clear()
            ctx.events.send("level_complete", {"level": 1})
        if ctx.time.frame == 5:
            game.count = ctx.scene.entity_count

    game = play(5, probe=probe)
    assert game.level == 2
    assert len(game.coins) == len(cb.COINS)
    level_one = cb.make_level(1)
    level_two = cb.make_level(2)
    assert level_two.entity_count == level_one.entity_count + 1
    assert game.count == level_two.entity_count


def test_pause_stops_the_player_sliding():
    class Pausing(cb.CookbookPlatformer):
        headless = True
        target_fps = 60

        def on_update(self, ctx):
            frame = ctx.time.frame
            if frame < 30:
                ctx.scene["player"].character_velocity = (cb.WALK_SPEED, -1.0, 0.0)
                return
            if frame == 30:
                self.paused = True
            super().on_update(ctx)
            if frame == 32:
                self.x_paused = ctx.scene["player"].transform.position[0]
            if frame == 60:
                self.x_later = ctx.scene["player"].transform.position[0]
                ctx.quit()

    game = Pausing()
    game.run()
    assert game.x_later == pytest.approx(game.x_paused, abs=0.05)
