# AUTOMATA
## General-Purpose 2D & 3D Game Engine

**Project type:** Open-source / independent game engine  
**Native language:** Rust  
**Scripting language:** Python  
**Graphics API:** wgpu  
**Physics:** Rapier  
**Python interoperability:** PyO3  
**Primary target:** Small-to-medium indie games

---

# 1. Vision

AUTOMATA is a lightweight, extensible, general-purpose 2D and 3D game engine designed for independent game development.

The objective is to provide a capable engine that allows developers to create different genres of games without requiring a large, complicated development environment.

AUTOMATA should prioritize:

- Performance and resource efficiency.
- Stability and predictable behavior.
- A clean and accessible Python API.
- Modular architecture.
- Reusability across different game genres.
- Cross-platform development.
- Incremental extensibility.

The engine should handle low-level operations through its native Rust implementation while exposing high-level functionality through Python.

The project should not attempt to replicate the entire feature set of Unreal Engine, Unity, or other large commercial engines.

## 1.1 Target Games

AUTOMATA should eventually be capable of developing:

**2D**
- Top-down RPGs.
- JRPGs.
- Platformers.
- 2D hack-and-slash games.
- Top-down action games.
- Roguelikes and roguelites.

**3D**
- Isometric RPGs.
- Third-person action RPGs.
- Persona-style JRPGs.
- First-person shooters.
- Third-person shooters.
- 3D hack-and-slash games.
- Exploration and adventure games.

The engine must remain genre-agnostic. Genre-specific functionality should be implemented through reusable modules and game-level code.

---

# 2. Core Technology Stack

| Component | Technology | Responsibility |
|---|---|---|
| Native core | Rust | Engine implementation |
| Scripting | Python 3 | Gameplay and high-level APIs |
| Python bindings | PyO3 | Rust/Python interoperability |
| Graphics | wgpu | GPU rendering |
| Window management | winit | Window creation and events |
| Mathematics | glam | Vectors, matrices, quaternions |
| Physics | Rapier | 2D and 3D physics |
| Audio | Kira | Audio playback and management |
| Asset formats | glTF | 3D models and scenes |
| Image loading | image | Texture and image processing |
| Serialization | serde | Configuration and data serialization |
| Build system | Cargo | Native compilation |
| Profiling | Tracy | Performance analysis |

Dependencies should be evaluated before adoption. Prefer maintained libraries with established APIs and avoid implementing existing infrastructure without a concrete reason.

---

# 3. Architecture

AUTOMATA should use a layered architecture.

## 3.1 Native Engine Core

Implemented entirely in Rust.

Responsibilities:

- Application lifecycle.
- Engine initialization and shutdown.
- Main game loop.
- Time management.
- Memory and resource management.
- Event handling.
- Logging and diagnostics.
- Native subsystem coordination.

## 3.2 Rendering System

A unified rendering architecture supporting both 2D and 3D.

**2D rendering**
- Sprites.
- Sprite sheets.
- Tilemaps.
- 2D cameras.
- Layers and sorting.
- 2D lighting.
- Particle systems.

**3D rendering**
- Meshes.
- Materials.
- Textures.
- Perspective and orthographic cameras.
- Lighting.
- Shadows.
- Model loading.
- Skyboxes.
- Post-processing.
- Particle systems.

Rendering should use wgpu and support modern graphics hardware.

Rendering should be modular so that new techniques can be introduced without redesigning the entire engine.

## 3.3 Scene Management

The scene system manages the game world.

Responsibilities:

- Scene creation and destruction.
- Entity management.
- Transform hierarchies.
- Scene transitions.
- Entity lifecycle.
- Serialization.
- Scene loading and unloading.

Scenes should support both 2D and 3D content.

## 3.4 Entity and Component System

Implement a lightweight ECS architecture.

Core concepts:

- Entities.
- Components.
- Systems.
- Queries.
- Entity lifecycle.

Avoid excessive abstraction and unnecessary complexity.

The ECS should support efficient iteration and predictable memory usage.

Evaluate an established ECS library before implementing a custom one.

## 3.5 Input System

Support:

- Keyboard.
- Mouse.
- Gamepads.
- Input mapping.
- Action-based input.
- Configurable controls.
- Input events.

Input should be independent of game genre.

## 3.6 Physics System

Use Rapier for physics simulation.

**2D**
- Rigid bodies.
- Colliders.
- Collision detection.
- Raycasting.
- Character controllers.

**3D**
- Rigid bodies.
- Colliders.
- Gravity.
- Collision detection.
- Raycasting.
- Character controllers.
- Physics queries.

Physics should be integrated into the engine lifecycle rather than directly exposed as an implementation detail to game scripts.

## 3.7 Animation System

**2D**
- Sprite animation.
- Animation sequences.
- Animation state management.

**3D**
- Skeletal animation.
- Bone hierarchies.
- Animation clips.
- Animation blending.
- Animation state machines.
- Root motion.

Animation is particularly important for action games, RPGs and FPS games.

## 3.8 Audio System

Use Kira for audio infrastructure.

Features:

- Sound effects.
- Music playback.
- Audio buses.
- Volume control.
- Spatial audio.
- Audio sources.
- Audio resource management.

## 3.9 Asset Management

Provide a centralized resource management system.

Supported resources:

- Textures.
- Models.
- Materials.
- Audio.
- Fonts.
- Animations.
- Shaders.
- Scenes.

Requirements:

- Resource caching.
- Resource handles.
- Asynchronous loading where appropriate.
- Resource lifetime management.
- Error handling.
- Asset unloading.

Prefer established formats such as glTF for 3D assets.

## 3.10 Python Scripting API

Python is the primary user-facing programming interface.

Rust functionality should be exposed through PyO3.

The Python API should provide intuitive access to:

- Game initialization.
- Scene management.
- Entities and components.
- Transforms.
- Cameras.
- Rendering objects.
- Physics.
- Audio.
- Input.
- Animation.
- Resources.

Example API (illustrative):

```python
import automata as at

game = at.Game(
    title="Project Zero",
    resolution=(1920, 1080),
)

scene = at.Scene()

player = at.Entity("Player")
player.add(at.Camera())
player.add(at.FirstPersonController())

scene.add(player)

game.load_scene(scene)
game.run()
```

Python should handle gameplay logic and orchestration.

Performance-critical operations should remain in Rust.

The native API should have clear ownership and lifetime rules. Avoid exposing unsafe native implementation details to Python.

---

# 4. Development Roadmap

Development should follow incremental milestones.

Each milestone must produce a working, testable result before proceeding.

## Phase 1 — Project Foundation

Objectives:

- Establish the repository.
- Configure Cargo workspace.
- Configure Rust development environment.
- Establish dependency management.
- Define coding conventions.
- Configure logging and error handling.
- Establish testing infrastructure.
- Create a minimal application entry point.

Deliverable:

A working Rust application with a defined lifecycle.

## Phase 2 — Window and Runtime

Objectives:

- Integrate winit.
- Create and manage application windows.
- Handle window events.
- Implement the main game loop.
- Implement frame timing.
- Implement input event collection.
- Integrate wgpu.

Deliverable:

A windowed application capable of running a stable update loop.

## Phase 3 — Rendering Foundation

Objectives:

- Initialize the graphics device.
- Implement render surfaces.
- Implement shaders.
- Implement vertex and index buffers.
- Implement textures.
- Implement basic materials.
- Implement a basic camera.
- Implement 2D rendering.
- Implement 3D rendering.

Deliverable:

A working renderer capable of displaying 2D and 3D objects.

## Phase 4 — Core Engine Architecture

Objectives:

- Implement scene management.
- Implement entities.
- Integrate or implement ECS.
- Implement transforms.
- Implement resource handles.
- Implement resource management.
- Implement event dispatching.
- Implement application lifecycle integration.

Deliverable:

A functional scene-based engine runtime.

## Phase 5 — Python Integration

Objectives:

- Integrate PyO3.
- Create the native Python extension.
- Expose core engine functionality.
- Implement Python-side abstractions.
- Establish Python/Rust ownership rules.
- Implement Python gameplay callbacks.
- Implement error propagation.

Deliverable:

A Python application capable of initializing AUTOMATA and controlling its runtime.

## Phase 6 — Physics and Input

Objectives:

- Integrate Rapier 2D.
- Integrate Rapier 3D.
- Implement collision queries.
- Implement raycasting.
- Implement character controllers.
- Implement input mappings.
- Implement gamepad support.

Deliverable:

A playable scene with controllable characters and physics interactions.

## Phase 7 — Assets, Animation and Audio

Objectives:

- Implement model loading.
- Implement texture loading.
- Implement material management.
- Implement skeletal animation.
- Implement sprite animation.
- Integrate audio.
- Implement resource caching.
- Implement asynchronous resource loading.

Deliverable:

A scene containing animated characters, models, textures and audio.

## Phase 8 — Higher-Level Game Systems

Develop reusable systems that build on the engine core.

Potential modules:

- UI framework.
- Dialogue framework.
- Inventory.
- Quest management.
- Save/load.
- Navigation.
- AI utilities.
- Particle effects.
- Timeline and cutscene systems.
- Localization.

These modules must remain optional and independent of the engine's core runtime.

Deliverable:

A more complete game-development framework.

## Phase 9 — Optimization and Stability

Objectives:

- Profile CPU performance.
- Profile GPU performance.
- Identify memory bottlenecks.
- Reduce unnecessary allocations.
- Improve rendering efficiency.
- Improve resource loading.
- Improve frame pacing.
- Test multithreaded workloads.
- Add stress tests.
- Test long-running applications.

Optimization must be driven by profiling and measurable performance improvements.

Deliverable:

A stable and profiled engine suitable for actual game development.

## Phase 10 — First Complete Game

Build a complete, small game using only AUTOMATA's public API.

The first game should validate:

- Scene management.
- Rendering.
- Input.
- Physics.
- Audio.
- Animation.
- UI.
- Resource management.
- Python scripting.
- Save/load.

Deliverable:

A playable and distributable game.

---

# 5. Acceptance Tests: Target Games

AUTOMATA should be evaluated through increasingly demanding projects.

These projects are validation targets, not requirements for the initial MVP.

## Project A — 2D RPG

Target: Pokémon-style game.

Required capabilities:

- Tilemaps.
- Character sprites.
- Sprite animations.
- NPCs.
- Dialogue.
- Inventory.
- Turn-based combat.
- Save/load.
- Scene transitions.

## Project B — 3D Isometric RPG

Required capabilities:

- 3D environments.
- Orthographic camera.
- Character movement.
- Navigation.
- Skeletal animation.
- NPC interaction.
- Dialogue.
- Inventory.
- Combat.
- Lighting.
- Save/load.

## Project C — 3D Action RPG

Required capabilities:

- Third-person camera.
- Character controller.
- Skeletal animation.
- Animation blending.
- Combat and hit detection.
- Weapons and abilities.
- Enemy AI.
- Physics.
- Particle effects.
- Audio.

## Project D — 3D FPS

Required capabilities:

- First-person camera.
- Mouse look.
- Character movement.
- Collision.
- Weapons.
- Raycasting.
- Projectile systems.
- Enemy AI.
- Animation.
- Audio.
- Level management.

---

# 6. Performance Goals

Performance is a core design consideration.

Initial targets should be measurable and hardware-dependent.

Suggested baseline:

- Stable 60 FPS on reasonable desktop hardware.
- Support for 120 FPS and higher where hardware permits.
- Predictable frame timing.
- Minimal unnecessary allocations during gameplay.
- Efficient resource caching.
- Efficient CPU/GPU workload distribution.
- Multithreading where profiling justifies it.

Performance budgets should be established for representative workloads rather than arbitrary maximum entity counts.

Use profiling tools before introducing optimization complexity.

---

# 7. Engineering Principles

The following principles should guide development.

1. **Keep the core small.** Do not implement unnecessary systems.
2. **Prefer existing infrastructure.** Use mature libraries instead of rewriting established functionality.
3. **Optimize based on evidence.** Avoid premature optimization.
4. **Keep modules independent.** Systems should be replaceable where practical.
5. **Maintain a clean Python API.** Game developers should not need to understand Rust internals.
6. **Preserve backward compatibility.** Avoid unnecessarily breaking the public API.
7. **Test every subsystem.** Include unit, integration and runtime tests.
8. **Document public functionality.** Every public API should have usable documentation.
9. **Build incrementally.** Maintain a working engine throughout development.
10. **Prioritize playable outcomes.** Engine development should ultimately enable actual games.

---

# 8. Explicit Non-Goals

AUTOMATA is not intended to become:

- A replacement for Unreal Engine.
- A replacement for Unity.
- A complete visual game editor in its initial stages.
- A custom operating system or graphics driver.
- A custom physics implementation.
- A full 3D modeling application.
- A complete game-specific RPG framework.
- A multiplayer networking platform in its initial stages.

Features should be introduced when they solve actual development problems.

---

# 9. Repository Structure

Proposed initial organization:

```text
automata/
├── Cargo.toml
├── pyproject.toml
├── README.md
├── LICENSE
│
├── crates/
│   ├── automata-core/
│   ├── automata-renderer/
│   ├── automata-scene/
│   ├── automata-input/
│   ├── automata-physics/
│   ├── automata-audio/
│   ├── automata-assets/
│   └── automata-python/
│
├── python/
│   └── automata/
│       ├── __init__.py
│       ├── game.py
│       ├── scene.py
│       └── components/
│
├── examples/
│   ├── window/
│   ├── triangle/
│   ├── 2d_scene/
│   ├── 3d_scene/
│   └── first_person/
│
├── tests/
│   ├── native/
│   └── python/
│
├── games/
│
└── docs/
    ├── architecture.md
    ├── api.md
    └── development.md
```

This structure is provisional. Introduce crates only when subsystem boundaries become meaningful.

---

# 10. AI-Assisted Development Workflow

AUTOMATA will be developed with substantial AI coding assistance.

AI should be treated as an implementation assistant, not as the authority on architecture.

Development rules:

- Maintain a clear project specification.
- Implement one subsystem at a time.
- Require AI to inspect existing code before modifying it.
- Avoid large, uncontrolled rewrites.
- Require tests for new functionality.
- Verify dependency APIs against official documentation.
- Review generated Rust ownership and concurrency decisions.
- Profile performance claims.
- Keep architectural decisions documented.
- Maintain working builds after each milestone.

Do not allow generated code to introduce unnecessary abstractions or dependencies.

Every completed subsystem must have a defined public interface and tests.

---

# 11. Definition of Success

AUTOMATA will be considered successful when:

1. It can create and run both 2D and 3D games.
2. Developers can write gameplay logic in Python.
3. The native runtime provides reliable performance.
4. Core systems work together without excessive manual integration.
5. The engine can support different game genres.
6. It can produce complete, playable and distributable indie games.
7. Its architecture can evolve without requiring fundamental rewrites.

**The ultimate objective is not to build the most sophisticated engine. It is to build an engine capable of producing the games envisioned for it.**

---

**AUTOMATA**

*One engine. Multiple genres. Multiple dimensions.*
