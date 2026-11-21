import automata as at

COINS = {"coin_0": -1.5, "coin_1": 1.0, "coin_2": 3.5}


class Level(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(at.Entity("cam").add(at.Camera3D(position=(0, 3, 9), target=(0, 0.5, 0))))
        player = at.Entity("player").add(at.Transform(position=(-4, 0.5, 0)))
        self.add(player.add(at.MeshRenderer.cube(size=0.8, color=(0.95, 0.45, 0.15, 1.0))))
        for name, x in COINS.items():
            coin = at.Entity(name).add(at.Transform(position=(x, 0.5, 0)))
            self.add(coin.add(at.MeshRenderer.cube(size=0.4, color=(1.0, 0.85, 0.25, 1.0))))


class CoinGame(at.Game):
    title = "module 8: resources"

    def on_start(self):
        self.load_scene(Level())
        self.x = -4.0
        self.coins = dict(COINS)
        self.controls = at.ActionMap()
        self.controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))

    def on_update(self, ctx):
        if "score" not in ctx.resources:
            ctx.resources.set("score", 0)
        self.x += self.controls.axis("move_x", ctx.input) * 4.0 * ctx.time.delta
        ctx.scene["player"].transform.position = (self.x, 0.5, 0.0)
        for name, cx in list(self.coins.items()):
            if abs(self.x - cx) < 0.6:  # the player grabs a coin
                ctx.scene[name].despawn()
                del self.coins[name]
                score = ctx.resources.get("score") + 1
                ctx.resources.set("score", score)  # takes effect immediately
                ctx.events.send("coin_pickup", {"score": score})
                if not self.coins:
                    ctx.events.send("all_collected", {"score": score})

    def on_coin_pickup(self, payload):      # auto-registered by name
        print(f"score: {payload['score']}")

    def on_all_collected(self, payload):    # a second handler, a second event
        print(f"all {len(COINS)} coins! final score {payload['score']}")


CoinGame().run()
