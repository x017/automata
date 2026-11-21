from oop_platformer import Coin, Platformer


def test_collecting_a_coin_despawns_it():
    class Probe(Platformer):
        headless = True

        def on_update(self, ctx):
            if ctx.time.frame == 1:  # drop a coin right on the player
                ctx.scene.spawn(Coin("coin_near", x=-4.0, y=1.4))
                self.coins["coin_near"] = (-4.0, 1.4)
            super().on_update(ctx)
            if ctx.time.frame == 60:
                self.found = ctx.scene.find("coin_near")
                ctx.quit()

    game = Probe()
    game.run()
    assert game.found is None
    assert "coin_near" not in game.coins
    assert game.score == 1
