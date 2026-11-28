# Custom shaders

Write your own [WGSL](https://www.w3.org/TR/WGSL/) to change how a surface
looks (**material shaders**) or how the whole frame looks (**post
shaders**). Both are plain `.wgsl` files in your project. Save one and the
scene changes at once.

You don't write a whole GPU shader. You write a single function that gets
fixed inputs and returns a result, and the engine does everything around it:
bindings, lighting, shadows, fog, the web build. The engine checks a shader
each time it reads it. A broken shader never reaches the GPU. Instead the
Console says what's wrong and on which line, and the scene keeps its
standard look until you fix it.

| Kind | You write | Used by |
|---|---|---|
| Material | `fn material(in: MaterialInput) -> MaterialOutput` | a [material file](materials.md)'s **Shader**: every object wearing that material |
| Post | `fn post(in: PostInput) -> vec3<f32>` | a **Post Shader** component: the whole frame |

## Making one

- **File ▸ New Material Shader…** or **New Post Shader…** (also under **+**
  in the Files panel). Either one starts a file from a template that
  already works, then opens it in the [code editor](code-editor.md).
- In the material editor, **Shader ▸ Choose…** picks a `.wgsl`. If you type
  a name that doesn't exist yet, Oberon makes the file from the template.
- Click the **Shader** row of a Post Shader component. It works the same
  way.

New shaders go in `shaders/` by default.

While you type, the code editor runs the same checks the engine does. Errors
are underlined, with the message on hover, before you even save. WGSL gets
syntax highlighting.

## Material shaders

```wgsl
// Glows at the edges, like a soap bubble.
fn material(in: MaterialInput) -> MaterialOutput {
    var out = material_defaults(in);           // the standard surface
    let rim = pow(1.0 - abs(dot(in.normal, in.view)), 2.0);
    out.emissive = out.emissive + in.p0.rgb * rim * in.p0.a;
    return out;
}
```

Your function runs for every pixel of every object wearing the material. It
runs after the standard material has been read (colour, textures, tiling)
and before lighting. So what you return is lit, shadowed and fogged like
any other surface.

**`MaterialInput`**

| Field | Type | What |
|---|---|---|
| `position` | `vec3<f32>` | world position (metres) |
| `normal` | `vec3<f32>` | world normal, unit length, normal map applied |
| `view` | `vec3<f32>` | unit vector toward the camera |
| `uv` | `vec2<f32>` | texture coordinates, with the material's tiling and offset |
| `pixel` | `vec2<f32>` | window position in pixels (top-left origin) |
| `color` | `vec4<f32>` | base colour × base map (alpha in `a`) |
| `roughness`, `metallic` | `f32` | the material's (maps applied) |
| `emissive` | `vec3<f32>` | the material's glow |
| `time` | `f32` | seconds since the scene started |
| `p0`, `p1` | `vec4<f32>` | the material's **shader numbers** (below) |

**`MaterialOutput`** has `color` (`vec4`, where alpha below 1 is
see-through), `normal`, `roughness`, `metallic` and `emissive`. Start from
`material_defaults(in)` and change only what you need. `discard` cuts a
pixel out (see the dissolve example below).

`base_map(uv)` reads the material's base colour texture anywhere, untinted.
It also works inside branches and loops.

## Post shaders

```wgsl
// Black and white, with a little warmth left in.
fn post(in: PostInput) -> vec3<f32> {
    let grey = vec3<f32>(dot(in.color, vec3<f32>(0.2126, 0.7152, 0.0722)));
    return mix(grey, in.color, in.p0.x);
}
```

Your function runs once per pixel of the finished frame. That is after tone
mapping, [grading](graphics.md) and anti-aliasing, and before game menus
and the HUD (those stay crisp). Colours are linear RGB in 0..1, and the
engine encodes them for the screen afterwards.

**`PostInput`**

| Field | Type | What |
|---|---|---|
| `uv` | `vec2<f32>` | where on the screen, 0..1 (top-left origin) |
| `pixel` | `vec2<f32>` | the same in pixels |
| `color` | `vec3<f32>` | the finished image here |
| `resolution` | `vec2<f32>` | the image size in pixels |
| `time` | `f32` | seconds since the scene started |
| `p0`, `p1` | `vec4<f32>` | the component's **params** |

Use `image(uv)` to read the image anywhere else (blur, warp, fringes) and
`image_pixel(pixel)` to read it at an exact pixel (edge detection). Every
pixel must get a colour, so `discard` isn't allowed here.

**Using one:** select any entity and use **Add Component ▸ Post Shader**,
then pick the file. You can also drag a `.wgsl` from the Files panel onto
an entity, or right-click the file and choose **Use as *Entity*'s Post
Effect** (switching files keeps the numbers). The first entity with a Post Shader draws it. It sits on top of
the [Environment](graphics.md)'s own effects (bloom, vignette, outlines),
which still apply.

## Numbers: `p0` and `p1`

Each shader gets up to **8 numbers** you can tune without editing code:

- `p0` holds the first four (`p0.x`, `p0.y`, `p0.z`, `p0.w`, or `p0.rgb` /
  `p0.a` for a colour). `p1` holds the next four.
- Missing numbers are 0.
- Material numbers are set in the material editor's **Shader** section.
- Post numbers are set in the component's **p0** and **p1** rows.
- Both can be dragged live.

Put a comment at the top of the shader saying what each number does. The
templates and examples all do.

## Helpers

Both kinds can call:

| Function | What |
|---|---|
| `noise3(p: vec3<f32>) -> f32` | smooth value noise in 0..1 |
| `hash1(n: f32) -> f32` | a pseudo-random number in 0..1 from any number |

You can add your own functions, structs and constants freely, as long as
the names don't clash with the engine's. The message says if one does.

## The rules

These are checked on every load:

- Define the function, with exactly the signature above.
- No `@group`/`@binding`, entry points (`@vertex`, `@fragment`,
  `@compute`), or other attributes. The engine owns those.
- No global variables (`var<private>`, `var<uniform>`, …), only locals
  inside functions. No `override`, `enable`, `requires` or `diagnostic`.
- It must also translate for WebGL2, so it runs in [web
  exports](exporting.md).
- At most 64 KB.

An error looks like `shaders/glow.wgsl:12:9: no definition in scope for
identifier: 'colr'`. It shows in the Console and in the log (and in the
material editor, where the shader row turns red). When you save a fix, the
Console says `shaders/glow.wgsl: shader reloaded`.

## In the scene file

```ron
// materials/glow.mat.ron (paths relative to the material file)
(
    color: (1.0, 1.0, 1.0, 1.0),
    shader: "../shaders/glow.wgsl",
    shader_params: [0.25, 0.85, 1.0, 3.0],
)
```

```ron
// an entity in a scene (paths relative to the project)
post_shader: Some((shader: "shaders/crt.wgsl", params: [0.08, 0.35, 1.5, 0.6])),
```

## From Python

```python
import automata as at

entity.add(at.PostShader("shaders/crt.wgsl", params=[0.08, 0.35, 1.5, 0.6]))

at.check_shader("shaders/glow.wgsl")  # None, or "file:line:col: message"
```

`check_shader` runs the engine's checks without a GPU, which is handy in
tests. Material shaders are named in the `.mat.ron` file, and
`MeshRenderer(material=...)` picks those up.

## Exporting

`automata check` and every export check the shaders the scene uses:

- **A material's shader:** if it breaks the rules, that's an error, since
  the objects wearing it would draw plain.
- **A post shader:** if it's missing, outside the project, or broken,
  that's also an error.

## The Shader Lab

`examples/oberon/shader_lab/` has three showpieces with material shaders and
three post effects to switch between:

| Shader | Kind | Shows |
|---|---|---|
| `hologram` | material | see-through, a bright rim, rolling scanlines, flicker |
| `dissolve` | material | burning away through noise with a glowing edge (`discard`) |
| `lava` | material | drifting crust over glowing rock, all from `noise3` |
| `crt` | post | a curved old monitor: scanlines, colour fringes, vignette |
| `ink` | post | comic outlines (edge detection with `image_pixel`) and flat colour bands |
| `underwater` | post | a wavering, blue-green view (`image` with offsets) |

```console
cargo run -p oberon --release examples/oberon/shader_lab/level.ron
```

Open `shaders/lava.wgsl`, change `0.9, 0.3, 0.05` to another colour and
save. To switch the post effect, drag `shaders/ink.wgsl` onto the **Look**
entity. Its numbers stay as they were, so set **p0** to `0.85, 0.12, 4, 0`
(each shader's header says what its numbers do; `build_level.py` lists a
good start for each in `POST_EFFECTS`).
