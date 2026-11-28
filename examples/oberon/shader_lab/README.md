# Shader Lab

Three showpieces wearing [custom material shaders](../../../docs/wiki/oberon/shaders.md),
seen through a post effect shader. Every look here is a short `.wgsl` file
in `shaders/`.

```console
cargo run -p oberon --release examples/oberon/shader_lab/level.ron
```

| Shader | Kind | Used by | Shows |
|---|---|---|---|
| `hologram` | material | Hologram | see-through, a bright rim, rolling scanlines, flicker |
| `dissolve` | material | Dissolving Crate | burning away and back through noise, with a glowing edge (`discard`) |
| `lava` | material | Lava Ball | drifting crust over glowing rock, all from `noise3` |
| `crt` | post | Look (the default) | a curved old monitor: scanlines, colour fringes, vignette |
| `ink` | post | | comic outlines (edge detection) and flat colour bands |
| `underwater` | post | | a wavering, blue-green view |

Each shader starts with a comment saying what its numbers (`p0`, `p1`) do.
The materials set theirs in `materials/*.mat.ron` (`shader_params`), and the
**Look** entity sets the post effect's in its Post Shader component.

Things to try:

- Open `shaders/lava.wgsl` (double-click it in the Files panel), change
  `0.9, 0.3, 0.05` to another colour and save. The ball changes at once.
- Open `materials/dissolve.mat.ron` in the material editor and set the
  second number to 0. Then drag the first one between 0 and 1 to burn the
  crate by hand.
- Drag `shaders/ink.wgsl` onto **Look** and set its **p0** to
  `0.85, 0.12, 4, 0`. For `underwater`, use `p0 = 3, 1.5, 0, 0` and
  `p1 = 0.45, 0.85, 0.9, 0.6`.
- Break something: delete a `;`. The Console names the file, line and
  column, the code editor underlines it, and the object falls back to its
  standard look until you fix it.

`build_level.py` writes the materials and the scene. The shaders are
hand-written.

```console
python examples/oberon/shader_lab/build_level.py
pytest examples/oberon/shader_lab/tests
```
