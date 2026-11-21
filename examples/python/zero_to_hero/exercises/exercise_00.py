import automata as at

game = at.Game(title="module 0")
frames = game.run(lambda ctx: None, max_frames=120)
print(f"ran {frames} frames")
