# Prefabs & attributes

## Attributes

Attributes are named values that you set per entity in the Inspector and
read from scripts. Use them to tune gameplay without touching code: a
coin's value, an enemy's speed, a door's key colour.

**Inspector → Attributes → + Add attribute** asks for a name and a type:

| Type | Python value | Example |
|---|---|---|
| Bool | `True` / `False` | `locked` |
| Int | `int` | `value = 10` |
| Float | `float` | `speed = 3.5` |
| Text | `str` | `next_level = "cave.ron"` |
| Vec3 | `(x, y, z)` tuple | `patrol_to` |
| Color | `(r, g, b, a)` tuple | `tint` |

In a script, `entity.attributes` works like a dict:

```python
def update(entity, ctx):
    speed = entity.attributes.get("speed", 2.0)
    entity.transform.translate((speed * ctx.time.delta, 0, 0))

    hp = entity.attributes["health"]
    if hp <= 0:
        entity.attributes["dead"] = True     # writes back like any other change
```

- Attributes are saved in the scene file, so they're part of the level
  data.
- Writes made during Play are visible to other scripts on the next
  frame. They're thrown away on Stop, like every other Play change.
- `entity.state` is the place for private values your script doesn't
  want to show in the Inspector.

## Prefabs

A prefab is an entity saved to its own file (`.prefab`) so you can stamp
copies of it into any scene.

- **Save:** right-click an entity → **Save as Prefab…**, or **File → Save
  Selection as Prefab…**. The entity's components, attributes, and
  attached script all go into the file.
- **Use:** **File → Instantiate Prefab…** drops a copy on the ground at
  the point the view is looking at.

Prefab files are plain RON scene snippets. You can diff them, review
them, and generate them.

### From Python

```python
import automata as at

coin = at.Prefab.load("prefabs/coin.prefab")

scene = at.Scene()
for i in range(10):
    scene.add(coin.instantiate(position=(i * 2.0, 1.0, 0.0)))

# or at runtime, from a game update:
def update(ctx):
    if ctx.input.just_pressed(at.Key.SPACE):
        ctx.scene.spawn(coin, position=(0, 5, 0))
```

> Spawning from **Oberon behaviour scripts** isn't supported in the
> editor's Play mode yet. Place prefabs in the scene and enable or move
> them from scripts instead, or run the scene with `at.run_scene`, where
> the full engine API is available through `ctx.raw`.
