import pytest

from oop_platformer import Platformer  # the Game subclass from your game module


def test_player_lands_on_the_ground():
    class Probe(Platformer):
        """The real game, plus a test probe in its update hook."""

        headless = True

        def on_update(self, ctx):
            super().on_update(ctx)  # real game logic runs unchanged
            if ctx.time.frame == 120:
                self.landed_y = ctx.scene["player"].transform.position[1]
                ctx.quit()

    game = Probe()
    game.run()
    assert game.landed_y == pytest.approx(1.4, abs=0.1)  # floor top 0.5 + capsule 0.9
