# Night Lights

A small night scene that shows placed lights. A **Point Light** torch hangs
on a stone wall. A car parked facing the wall has two **Spot Light**
headlights. A crate and a post stand in the light. None of the light comes
from a glowing mesh: the flame and the lenses glow for the look but have
`glow_light=0`. All three lights cast shadows on the High preset and above.

```bash
python examples/oberon/night_lights/build_level.py        # rewrites level.ron
cargo run -p oberon --release examples/oberon/night_lights/level.ron
pytest examples/oberon/night_lights/tests
```

Things to try in Oberon:

- Select a headlight and use the **Rotate** tool to swing the beam. The
  viewport draws its cone.
- Change the headlights' **Cone Angle** and **Inner Angle**. A small gap
  between the two gives a hard-edged beam; a large gap gives a soft one.
- Untick **Cast Shadows** on the torch and watch the crate's shadow
  disappear.
- Switch the Graphics preset to Medium. Lights still shine there, but
  they cast no shadows.

Guide: [Rendering & Lights → Point lights and spotlights](../../../docs/wiki/guides/rendering-and-lights.md#point-lights-and-spotlights).
