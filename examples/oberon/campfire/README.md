# Campfire

A campfire at night that shows **particles**. Three Particle Emitters
sit on the log pile:

- **Flames** use the `fire` preset.
- **Smoke** uses the `smoke` preset. It rises above the flames and is lit,
  so the firelight colours it.
- **Embers** start from the `sparks` preset, changed into a slow trickle
  that rises.

The `scripts/crackle.py` script makes the fire pop now and then. It throws
extra embers with `ctx.particles.emit`, and books the next pop with
`ctx.after`. The flames don't light anything
themselves. A Point Light in the fire lights the stones, logs and trees
around it.

```bash
python examples/oberon/campfire/build_level.py        # rewrites level.ron
cargo run -p oberon --release examples/oberon/campfire/level.ron
pytest examples/oberon/campfire/tests
```

Things to try in Oberon:

- Select **Flames** and change the **Preset** to Magic, then back to Fire.
  The scene view plays the particles while you edit.
- Raise the smoke's **Rate** and **End Size** for a smoky, damp-wood fire.
- Turn up the flames' **Start Glow** on the Beautiful preset to make them
  bloom.
- Press Play and watch the embers pop.

Guide: [Rendering & Lights → Particles](../../../docs/wiki/guides/rendering-and-lights.md#particles).
