//! Particle emitters: fire, smoke, sparks, dust, splashes, magic.
//!
//! A [`ParticleEmitter`] is plain data saved with the scene: how many
//! particles to make, where, how they move, and how they look over their
//! life. [`simulate_particles`] runs them on the CPU each frame (keeping
//! each emitter's live particles in a [`ParticleState`] beside it), and
//! [`crate::record_draw_list`] hands them to the renderer as one
//! [`automata_renderer::ParticleBatch`] per emitter.
//!
//! [`ParticleEmitter::preset`] gives ready-made looks; [`spawn_burst`]
//! fires a one-shot effect at a point (sparks where a shot hits), which
//! cleans itself up when its particles are gone.

use automata_renderer::{ParticleBatch, ParticleBlend, ParticleInstance, ParticleSprite, Texture};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::{GlobalTransform, SceneWorld, Transform};

/// Where an emitter's particles start, in its entity's space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum EmitterShape {
    /// All from the entity's position.
    Point,
    /// Anywhere in a ball of radius `shape_size.x`, flying outward.
    Sphere,
    /// Anywhere in a box of half extents `shape_size`.
    Box,
    /// From a disc of radius `shape_size.x` facing up (+Y).
    #[default]
    Cone,
}

impl EmitterShape {
    /// Every shape.
    pub const ALL: [Self; 4] = [Self::Point, Self::Sphere, Self::Box, Self::Cone];

    /// Its name in the editor.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Point => "Point",
            Self::Sphere => "Sphere",
            Self::Box => "Box",
            Self::Cone => "Cone",
        }
    }
}

/// The built-in looks, by name.
pub const PRESETS: [&str; 6] = ["Fire", "Smoke", "Sparks", "Dust", "Splash", "Magic"];

/// Makes particles: a scene component (saved with the scene).
///
/// Particles leave within `cone_angle` of the entity's up (+Y) axis
/// (spheres fly outward instead), slow down by `drag`, fall by `gravity`
/// (negative rises, like smoke), and over their `lifetime` grow or shrink
/// from `size[0]` to `size[1]` and fade from `color` to `color_end`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // independent switches, as in the editor
pub struct ParticleEmitter {
    /// Making new particles (existing ones live out their lives either way).
    pub emitting: bool,
    /// New particles per second.
    pub rate: f32,
    /// Particles made all at once when it starts.
    pub burst: u32,
    /// Seconds it makes particles for; 0 = forever.
    pub duration: f32,
    /// Despawns the entity once it's done making particles and the last
    /// one is gone (one-off effects).
    pub one_shot: bool,
    /// The most alive at once.
    pub max_particles: u32,
    /// How long each lives: between `lifetime[0]` and `lifetime[1]` seconds.
    pub lifetime: [f32; 2],
    /// Where they start.
    pub shape: EmitterShape,
    /// The shape's size: a sphere's or cone's radius (x), a box's half
    /// extents (m).
    pub shape_size: [f32; 3],
    /// How far from straight up they may leave (degrees; 180 = any way).
    pub cone_angle: f32,
    /// Starting speed: between `speed[0]` and `speed[1]` m/s.
    pub speed: [f32; 2],
    /// Downward acceleration (m/s²; 9.81 falls like a stone, negative rises).
    pub gravity: f32,
    /// How quickly they slow down (per second; 0 = never).
    pub drag: f32,
    /// Size at birth and at death (m).
    pub size: [f32; 2],
    /// How much sizes vary (0 = all the same, 1 = down to nothing).
    pub size_spread: f32,
    /// Colour at birth (linear RGBA; over 1 glows on Beautiful).
    pub color: [f32; 4],
    /// Colour at death.
    pub color_end: [f32; 4],
    /// The share of its life a particle takes to fade in (0..1).
    pub fade_in: f32,
    /// The share of its life it takes to fade out at the end (0..1).
    pub fade_out: f32,
    /// The fastest they spin (radians per second, either way).
    pub spin: f32,
    /// Stretches particles back along their motion: the streak is this
    /// many seconds of travel (sparks; 0 = round).
    pub stretch: f32,
    /// How they blend: over the scene (smoke) or adding light (fire).
    pub blend: ParticleBlend,
    /// Lit by the sun and lights (smoke, dust), or glowing (fire, sparks).
    pub lit: bool,
    /// Fade over this many metres where they meet the scene behind (0 = a
    /// hard edge).
    pub soft: f32,
    /// The built-in sprite, when there's no `texture`.
    pub sprite: ParticleSprite,
    /// An image file to use instead of the built-in sprite (empty = none).
    pub texture: String,
    /// The image's frames, columns by rows; a sheet of more than one plays
    /// once over each particle's life.
    pub sheet: [u32; 2],
    /// Particles stay where they were made when the entity moves (smoke
    /// trails); off, they move with it.
    pub world_space: bool,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            emitting: true,
            rate: 20.0,
            burst: 0,
            duration: 0.0,
            one_shot: false,
            max_particles: 500,
            lifetime: [1.0, 2.0],
            shape: EmitterShape::Cone,
            shape_size: [0.1, 0.1, 0.1],
            cone_angle: 25.0,
            speed: [1.0, 2.0],
            gravity: 0.0,
            drag: 0.0,
            size: [0.3, 0.6],
            size_spread: 0.2,
            color: [1.0, 1.0, 1.0, 1.0],
            color_end: [1.0, 1.0, 1.0, 0.0],
            fade_in: 0.1,
            fade_out: 0.3,
            spin: 0.0,
            stretch: 0.0,
            blend: ParticleBlend::Alpha,
            lit: false,
            soft: 0.5,
            sprite: ParticleSprite::Dot,
            texture: String::new(),
            sheet: [1, 1],
            world_space: true,
        }
    }
}

impl ParticleEmitter {
    /// A built-in look by name, any case (see [`PRESETS`]): `"fire"`,
    /// `"smoke"`, `"sparks"` (a one-off burst), `"dust"` (motes drifting
    /// in a 8 x 3 x 8 m box), `"splash"` (a one-off burst) or `"magic"`.
    #[must_use]
    #[allow(clippy::too_many_lines)] // one table of looks
    pub fn preset(name: &str) -> Option<Self> {
        let base = Self::default();
        Some(match name.to_ascii_lowercase().as_str() {
            "fire" => Self {
                rate: 45.0,
                max_particles: 200,
                lifetime: [0.6, 1.1],
                shape_size: [0.25, 0.1, 0.25],
                cone_angle: 12.0,
                speed: [0.6, 1.4],
                gravity: -1.8,
                drag: 0.8,
                size: [0.6, 0.15],
                size_spread: 0.35,
                color: [2.0, 1.15, 0.4, 1.0],
                color_end: [1.2, 0.18, 0.03, 1.0],
                fade_in: 0.12,
                fade_out: 0.5,
                spin: 1.2,
                blend: ParticleBlend::Additive,
                soft: 0.3,
                sprite: ParticleSprite::Flame,
                ..base
            },
            "smoke" => Self {
                rate: 7.0,
                max_particles: 100,
                lifetime: [3.5, 5.5],
                shape_size: [0.2, 0.1, 0.2],
                cone_angle: 10.0,
                speed: [0.5, 0.9],
                gravity: -0.25,
                drag: 0.3,
                size: [0.5, 2.6],
                size_spread: 0.3,
                color: [0.3, 0.29, 0.28, 0.5],
                color_end: [0.55, 0.55, 0.56, 0.0],
                fade_in: 0.15,
                fade_out: 0.5,
                spin: 0.4,
                lit: true,
                soft: 0.8,
                sprite: ParticleSprite::Puff,
                ..base
            },
            "sparks" => Self {
                rate: 0.0,
                burst: 24,
                one_shot: true,
                max_particles: 200,
                lifetime: [0.25, 0.6],
                shape: EmitterShape::Point,
                cone_angle: 70.0,
                speed: [3.0, 7.0],
                gravity: 9.81,
                drag: 1.5,
                size: [0.07, 0.03],
                size_spread: 0.3,
                // Bright enough to read against a daylight sky.
                color: [8.0, 4.0, 1.2, 1.0],
                color_end: [4.0, 0.9, 0.1, 1.0],
                fade_in: 0.0,
                fade_out: 0.4,
                stretch: 0.03,
                blend: ParticleBlend::Additive,
                soft: 0.0,
                sprite: ParticleSprite::Spark,
                ..base
            },
            "dust" => Self {
                rate: 6.0,
                max_particles: 120,
                lifetime: [5.0, 8.0],
                shape: EmitterShape::Box,
                shape_size: [4.0, 1.5, 4.0],
                cone_angle: 180.0,
                speed: [0.02, 0.08],
                drag: 0.2,
                size: [0.04, 0.04],
                size_spread: 0.5,
                color: [1.0, 0.95, 0.85, 0.6],
                color_end: [1.0, 0.95, 0.85, 0.6],
                fade_in: 0.3,
                fade_out: 0.4,
                lit: true,
                soft: 0.0,
                ..base
            },
            "splash" => Self {
                rate: 0.0,
                burst: 40,
                one_shot: true,
                max_particles: 200,
                lifetime: [0.6, 1.1],
                shape_size: [0.3, 0.1, 0.3],
                cone_angle: 30.0,
                speed: [2.5, 5.0],
                gravity: 9.81,
                drag: 0.4,
                size: [0.12, 0.3],
                size_spread: 0.4,
                color: [0.85, 0.92, 1.0, 0.8],
                color_end: [0.85, 0.92, 1.0, 0.0],
                fade_in: 0.0,
                fade_out: 0.5,
                stretch: 0.02,
                lit: true,
                soft: 0.2,
                ..base
            },
            "magic" => Self {
                rate: 30.0,
                max_particles: 150,
                lifetime: [1.0, 1.8],
                shape: EmitterShape::Sphere,
                shape_size: [0.4, 0.4, 0.4],
                speed: [0.1, 0.4],
                gravity: -0.4,
                drag: 0.5,
                size: [0.18, 0.02],
                size_spread: 0.3,
                color: [0.6, 0.4, 2.0, 1.0],
                color_end: [1.5, 0.5, 2.0, 1.0],
                fade_in: 0.2,
                fade_out: 0.5,
                spin: 2.0,
                blend: ParticleBlend::Additive,
                soft: 0.0,
                sprite: ParticleSprite::Star,
                ..base
            },
            _ => return None,
        })
    }

    /// The preset this emitter is exactly (see [`PRESETS`]), if any.
    #[must_use]
    pub fn preset_name(&self) -> Option<&'static str> {
        PRESETS
            .into_iter()
            .find(|name| Self::preset(name).as_ref() == Some(self))
    }

    /// The sprite rectangle of `frame` of a particle `t` (0..1) through its
    /// life.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // small counts
    fn uv(&self, has_texture: bool, frame: u32, t: f32) -> [f32; 4] {
        if !has_texture {
            return self.sprite.uv(frame);
        }
        let (cols, rows) = (self.sheet[0].max(1), self.sheet[1].max(1));
        let frames = cols * rows;
        #[allow(clippy::cast_precision_loss)] // small counts
        let i = ((t * frames as f32) as u32).min(frames - 1);
        automata_renderer::sheet_cell(i % cols, i / cols, cols, rows)
    }
}

/// A value for [`ParticleEmitter::set_field`], as scripts send them.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// `True` / `False`.
    Bool(bool),
    /// A number.
    Number(f32),
    /// A tuple of numbers.
    Numbers(Vec<f32>),
    /// A name or path.
    Text(String),
}

impl ParticleEmitter {
    /// Every field's name, as scripts and scene files spell it.
    pub const FIELDS: [&'static str; 28] = [
        "emitting",
        "rate",
        "burst",
        "duration",
        "one_shot",
        "max_particles",
        "lifetime",
        "shape",
        "shape_size",
        "cone_angle",
        "speed",
        "gravity",
        "drag",
        "size",
        "size_spread",
        "color",
        "color_end",
        "fade_in",
        "fade_out",
        "spin",
        "stretch",
        "blend",
        "lit",
        "soft",
        "sprite",
        "texture",
        "sheet",
        "world_space",
    ];

    /// Sets field `name` (see [`Self::FIELDS`]) from a script's value,
    /// clamped to what makes sense. Pairs (`lifetime`, `speed`, `size`)
    /// take one number for both; colours take RGB or RGBA; names (`shape`,
    /// `blend`, `sprite`) any case.
    ///
    /// # Errors
    ///
    /// An unknown field, or a value of the wrong kind.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::too_many_lines,
        clippy::items_after_statements
    )] // counts; one arm per field
    pub fn set_field(&mut self, name: &str, value: &FieldValue) -> Result<(), String> {
        let wrong = || {
            let got = match value {
                FieldValue::Bool(b) => b.to_string(),
                FieldValue::Number(n) => n.to_string(),
                FieldValue::Numbers(v) => format!("{v:?}"),
                FieldValue::Text(t) => format!("{t:?}"),
            };
            format!("wrong kind of value for {name}: {got}")
        };
        let number = || match value {
            FieldValue::Number(v) if v.is_finite() => Ok(*v),
            _ => Err(wrong()),
        };
        let flag = || match value {
            FieldValue::Bool(b) => Ok(*b),
            FieldValue::Number(v) => Ok(*v != 0.0),
            _ => Err(wrong()),
        };
        let count = || number().map(|v| v.max(0.0).round() as u32);
        let numbers = |n: usize| match value {
            FieldValue::Number(v) => Ok(vec![*v; n]),
            FieldValue::Numbers(v) if v.len() >= n && v.iter().all(|x| x.is_finite()) => {
                Ok(v.clone())
            }
            _ => Err(wrong()),
        };
        let pair = || numbers(2).map(|v| [v[0], v[1]]);
        let color = || match value {
            FieldValue::Numbers(v) if v.len() == 3 => Ok([v[0], v[1], v[2], 1.0]),
            FieldValue::Numbers(v) if v.len() == 4 => Ok([v[0], v[1], v[2], v[3]]),
            _ => Err(wrong()),
        };
        let text = || match value {
            FieldValue::Text(s) => Ok(s.clone()),
            _ => Err(wrong()),
        };
        fn named<T: Copy>(
            text: &str,
            all: &[T],
            label: impl Fn(T) -> &'static str,
        ) -> Result<T, String> {
            all.iter()
                .copied()
                .find(|&x| label(x).eq_ignore_ascii_case(text))
                .ok_or_else(|| {
                    let options: Vec<String> =
                        all.iter().map(|&x| label(x).to_lowercase()).collect();
                    format!(
                        "unknown value {text:?}; expected one of {}",
                        options.join(", ")
                    )
                })
        }
        match name {
            "emitting" => self.emitting = flag()?,
            "rate" => self.rate = number()?.max(0.0),
            "burst" => self.burst = count()?,
            "duration" => self.duration = number()?.max(0.0),
            "one_shot" => self.one_shot = flag()?,
            "max_particles" => self.max_particles = count()?,
            "lifetime" => {
                let [a, b] = pair()?;
                self.lifetime = [a.max(0.01), b.max(a.max(0.01))];
            }
            "shape" => self.shape = named(&text()?, &EmitterShape::ALL, EmitterShape::label)?,
            "shape_size" => {
                let v = numbers(3)?;
                self.shape_size = [v[0].max(0.0), v[1].max(0.0), v[2].max(0.0)];
            }
            "cone_angle" => self.cone_angle = number()?.clamp(0.0, 180.0),
            "speed" => self.speed = pair()?,
            "gravity" => self.gravity = number()?,
            "drag" => self.drag = number()?.max(0.0),
            "size" => {
                let [a, b] = pair()?;
                self.size = [a.max(0.0), b.max(0.0)];
            }
            "size_spread" => self.size_spread = number()?.clamp(0.0, 1.0),
            "color" => self.color = color()?,
            "color_end" => self.color_end = color()?,
            "fade_in" => self.fade_in = number()?.clamp(0.0, 1.0),
            "fade_out" => self.fade_out = number()?.clamp(0.0, 1.0),
            "spin" => self.spin = number()?,
            "stretch" => self.stretch = number()?.max(0.0),
            "blend" => {
                self.blend = named(
                    &text()?,
                    &[ParticleBlend::Alpha, ParticleBlend::Additive],
                    blend_label,
                )?;
            }
            "lit" => self.lit = flag()?,
            "soft" => self.soft = number()?.max(0.0),
            "sprite" => {
                self.sprite = named(&text()?, &ParticleSprite::ALL, ParticleSprite::label)?;
            }
            "texture" => self.texture = text()?,
            "sheet" => {
                let v = numbers(2)?;
                self.sheet = [v[0].max(1.0).round() as u32, v[1].max(1.0).round() as u32];
            }
            "world_space" => self.world_space = flag()?,
            _ => return Err(format!("ParticleEmitter has no field {name:?}")),
        }
        Ok(())
    }
}

/// A blend mode's name in the editor and scripts.
#[must_use]
pub fn blend_label(blend: ParticleBlend) -> &'static str {
    match blend {
        ParticleBlend::Alpha => "Alpha",
        ParticleBlend::Additive => "Additive",
    }
}

/// One live particle.
#[derive(Debug, Clone, Copy)]
struct Particle {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    /// Its size relative to the emitter's.
    scale: f32,
    rotation: f32,
    spin: f32,
    /// Which sprite variant it shows.
    frame: u32,
}

/// An emitter's live particles (runtime only, not saved). Added beside each
/// [`ParticleEmitter`] by [`simulate_particles`].
#[derive(Debug, Clone, Default)]
pub struct ParticleState {
    particles: Vec<Particle>,
    /// Fractional particles owed by `rate` so far.
    carry: f32,
    /// Seconds since it started.
    age: f32,
    started: bool,
    /// Particles to make next step on top of the rate.
    pending: u32,
    rng: u64,
}

impl ParticleState {
    /// How many particles are alive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.particles.len()
    }

    /// Whether none are.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }

    /// Makes `count` extra particles on the next step.
    pub fn burst(&mut self, count: u32) {
        self.pending = self.pending.saturating_add(count);
    }

    /// Seconds since the emitter started.
    #[must_use]
    pub fn age(&self) -> f32 {
        self.age
    }

    /// The live particles' positions (world space for world-space
    /// emitters, the entity's otherwise).
    pub fn positions(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.particles.iter().map(|p| p.position)
    }

    /// A random number in [0, 1).
    fn random(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        #[allow(clippy::cast_precision_loss)] // 24 bits
        let v = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32;
        v / 16_777_216.0
    }

    fn between(&mut self, range: [f32; 2]) -> f32 {
        range[0] + (range[1] - range[0]) * self.random()
    }

    /// Whether the emitter has finished: it's stopped making particles and
    /// the last is gone.
    fn finished(&self, emitter: &ParticleEmitter) -> bool {
        let stopped = !emitter.emitting
            || emitter.rate <= 0.0
            || (emitter.duration > 0.0 && self.age >= emitter.duration);
        self.started && stopped && self.pending == 0 && self.particles.is_empty()
    }

    /// Makes one particle: position and velocity in the entity's space.
    fn spawn(&mut self, emitter: &ParticleEmitter, place: Mat4) {
        let size = Vec3::from(emitter.shape_size).max(Vec3::ZERO);
        let (local, outward) = match emitter.shape {
            EmitterShape::Point => (Vec3::ZERO, None),
            EmitterShape::Sphere => {
                let dir = self.unit_vector();
                let r = size.x * self.random().cbrt();
                (dir * r, Some(dir))
            }
            EmitterShape::Box => (
                Vec3::new(
                    (self.random() * 2.0 - 1.0) * size.x,
                    (self.random() * 2.0 - 1.0) * size.y,
                    (self.random() * 2.0 - 1.0) * size.z,
                ),
                None,
            ),
            EmitterShape::Cone => {
                let angle = self.random() * std::f32::consts::TAU;
                let r = size.x * self.random().sqrt();
                (Vec3::new(angle.cos() * r, 0.0, angle.sin() * r), None)
            }
        };
        let dir = outward.unwrap_or_else(|| self.in_cone(emitter.cone_angle));
        let speed = self.between(emitter.speed);
        let life = self.between(emitter.lifetime).max(0.01);
        let scale = 1.0 - emitter.size_spread.clamp(0.0, 1.0) * self.random();
        let spin = (self.random() * 2.0 - 1.0) * emitter.spin;
        let rotation = self.random() * std::f32::consts::TAU;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // 0..4096
        let frame = (self.random() * 4096.0) as u32;
        let (position, velocity) = if emitter.world_space {
            (
                place.transform_point3(local),
                place.transform_vector3(dir).normalize_or_zero() * speed,
            )
        } else {
            (local, dir * speed)
        };
        self.particles.push(Particle {
            position,
            velocity,
            age: 0.0,
            life,
            scale,
            rotation,
            spin,
            frame,
        });
    }

    /// A random direction, evenly over the sphere.
    fn unit_vector(&mut self) -> Vec3 {
        let z = self.random() * 2.0 - 1.0;
        let angle = self.random() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(angle.cos() * r, z, angle.sin() * r)
    }

    /// A random direction within `degrees` of +Y, evenly over that cap.
    fn in_cone(&mut self, degrees: f32) -> Vec3 {
        let cos_max = degrees.clamp(0.0, 180.0).to_radians().cos();
        let y = 1.0 - self.random() * (1.0 - cos_max);
        let angle = self.random() * std::f32::consts::TAU;
        let r = (1.0 - y * y).max(0.0).sqrt();
        Vec3::new(angle.cos() * r, y, angle.sin() * r)
    }

    /// Advances by `dt` seconds: ages and moves the particles, then makes
    /// new ones at `place` (the entity's world transform).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // counts
    fn step(&mut self, emitter: &ParticleEmitter, place: Mat4, dt: f32) {
        let dt = dt.max(0.0);
        // Gravity is along world down; local-space particles feel it in
        // their entity's frame.
        let down = if emitter.world_space {
            Vec3::NEG_Y
        } else {
            place
                .inverse()
                .transform_vector3(Vec3::NEG_Y)
                .normalize_or_zero()
        };
        let accel = down * emitter.gravity;
        let keep = 1.0 / (1.0 + emitter.drag.max(0.0) * dt);
        self.particles.retain_mut(|p| {
            p.age += dt;
            p.velocity = (p.velocity + accel * dt) * keep;
            p.position += p.velocity * dt;
            p.rotation += p.spin * dt;
            p.age < p.life
        });

        let mut count = std::mem::take(&mut self.pending);
        if self.started {
            self.age += dt;
        } else {
            self.started = true;
            count += emitter.burst;
        }
        let running = emitter.duration <= 0.0 || self.age < emitter.duration;
        if emitter.emitting && running && emitter.rate > 0.0 {
            self.carry += emitter.rate * dt;
            let whole = self.carry.floor();
            self.carry -= whole;
            count = count.saturating_add(whole as u32);
        }
        let room = (emitter.max_particles as usize).saturating_sub(self.particles.len());
        for _ in 0..(count as usize).min(room) {
            self.spawn(emitter, place);
        }
    }

    /// This frame's particles as the renderer draws them.
    fn batch(
        &self,
        emitter: &ParticleEmitter,
        place: Mat4,
        texture: Option<&Texture>,
    ) -> ParticleBatch {
        let ramp = |x: f32, width: f32| {
            if width <= 0.0 {
                1.0
            } else {
                (x / width).clamp(0.0, 1.0)
            }
        };
        let instances = self
            .particles
            .iter()
            .map(|p| {
                let t = (p.age / p.life).clamp(0.0, 1.0);
                let size = (emitter.size[0] + (emitter.size[1] - emitter.size[0]) * t) * p.scale;
                let mut color = [0.0; 4];
                for (i, c) in color.iter_mut().enumerate() {
                    *c = emitter.color[i] + (emitter.color_end[i] - emitter.color[i]) * t;
                }
                color[3] *= ramp(t, emitter.fade_in) * ramp(1.0 - t, emitter.fade_out);
                let (position, velocity) = if emitter.world_space {
                    (p.position, p.velocity)
                } else {
                    (
                        place.transform_point3(p.position),
                        place.transform_vector3(p.velocity),
                    )
                };
                ParticleInstance {
                    position: position.to_array(),
                    size: size.max(0.0),
                    color,
                    uv: emitter.uv(texture.is_some(), p.frame, t),
                    streak: (velocity * emitter.stretch).to_array(),
                    rotation: p.rotation,
                }
            })
            .collect();
        ParticleBatch {
            instances,
            blend: emitter.blend,
            lit: emitter.lit,
            soft: emitter.soft,
            texture: texture.cloned(),
        }
    }
}

/// A particle image loaded from [`ParticleEmitter::texture`] (runtime only).
#[derive(Debug, Clone)]
pub struct ParticleTexture {
    /// The file it came from.
    pub path: String,
    /// The image on the GPU.
    pub texture: Texture,
}

/// Runs every emitter for `dt` seconds: adds a [`ParticleState`] to new
/// ones, moves and ages their particles, makes new ones, and despawns
/// finished one-shot emitters.
pub fn simulate_particles(world: &mut SceneWorld, dt: f32) {
    step_emitters(world, dt, false);
}

/// [`simulate_particles`] for an editor's preview: emitters that finish
/// (one-shot bursts, say) play again after a short pause instead of
/// despawning, so the world keeps matching the document being edited.
pub fn preview_particles(world: &mut SceneWorld, dt: f32) {
    step_emitters(world, dt, true);
}

fn step_emitters(world: &mut SceneWorld, dt: f32, replay: bool) {
    if replay {
        let done: Vec<hecs::Entity> = world
            .query::<(hecs::Entity, &ParticleEmitter, &ParticleState)>()
            .iter()
            .filter(|(_, e, s)| s.finished(e) && s.age >= e.lifetime[1] + PREVIEW_PAUSE)
            .map(|(entity, _, _)| entity)
            .collect();
        for entity in done {
            let _ = world.remove_one::<ParticleState>(entity);
        }
    }
    let fresh: Vec<hecs::Entity> = world
        .query::<hecs::Without<hecs::With<hecs::Entity, &ParticleEmitter>, &ParticleState>>()
        .iter()
        .collect();
    for entity in fresh {
        let state = ParticleState {
            // Distinct, nonzero seeds per entity.
            rng: entity.to_bits().get().wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            ..ParticleState::default()
        };
        let _ = world.insert_one(entity, state);
    }
    // Emitters removed (in the editor, say) drop their particles.
    let orphans: Vec<hecs::Entity> = world
        .query::<hecs::Without<hecs::With<hecs::Entity, &ParticleState>, &ParticleEmitter>>()
        .iter()
        .collect();
    for entity in orphans {
        let _ = world.remove_one::<ParticleState>(entity);
    }
    let mut finished = Vec::new();
    for (entity, (emitter, state, global)) in &mut world.query::<(
        hecs::Entity,
        (
            &ParticleEmitter,
            &mut ParticleState,
            Option<&GlobalTransform>,
        ),
    )>() {
        let place = global.map_or(Mat4::IDENTITY, GlobalTransform::matrix);
        state.step(emitter, place, dt);
        if emitter.one_shot && !replay && state.finished(emitter) {
            finished.push(entity);
        }
    }
    for entity in finished {
        let _ = world.despawn(entity);
    }
}

/// Seconds a finished effect rests before an editor preview replays it.
const PREVIEW_PAUSE: f32 = 0.8;

/// Records every emitter's particles into `draw`, one batch each.
pub(crate) fn record_particles(world: &SceneWorld, draw: &mut automata_renderer::DrawList) {
    for (emitter, state, global, texture) in &mut world.query::<(
        &ParticleEmitter,
        &ParticleState,
        Option<&GlobalTransform>,
        Option<&ParticleTexture>,
    )>() {
        if state.is_empty() {
            continue;
        }
        let place = global.map_or(Mat4::IDENTITY, GlobalTransform::matrix);
        let texture = texture
            .filter(|t| t.path == emitter.texture)
            .map(|t| &t.texture);
        draw.particles(state.batch(emitter, place, texture));
    }
}

/// Loads emitters' image files (once per path) so their particles draw
/// with them; called with the renderer each frame, like
/// [`crate::materialize_visuals`]. Files that fail to load fall back to the
/// built-in sprite.
pub fn load_particle_textures(world: &mut SceneWorld, renderer: &automata_renderer::Renderer) {
    let wanted: Vec<(hecs::Entity, String)> = world
        .query::<(hecs::Entity, &ParticleEmitter, Option<&ParticleTexture>)>()
        .iter()
        .filter(|(_, e, t)| !e.texture.is_empty() && t.is_none_or(|t| t.path != e.texture))
        .map(|(entity, e, _)| (entity, e.texture.clone()))
        .collect();
    for (entity, path) in wanted {
        let texture = std::fs::read(&path)
            .ok()
            .and_then(|bytes| renderer.create_texture_from_bytes(&bytes).ok());
        let texture = texture.unwrap_or_else(|| {
            tracing::warn!(%path, "particle texture failed to load");
            // Remember the failure so it isn't retried every frame.
            renderer.white_texture()
        });
        let _ = world.insert_one(entity, ParticleTexture { path, texture });
    }
}

/// Spawns a one-off effect at `position`: `emitter` (a preset, say) set to
/// burst `count` particles (or its own `burst`; a second's worth for
/// continuous effects like fire) and despawn when they're gone. Its up axis turns to `direction` (sparks fly off a wall along its
/// normal).
pub fn spawn_burst(
    world: &mut SceneWorld,
    mut emitter: ParticleEmitter,
    position: Vec3,
    direction: Option<Vec3>,
    count: Option<u32>,
) -> hecs::Entity {
    // Continuous effects (fire, smoke) burst a second's worth by default.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a rate
    let second = emitter.rate.ceil().clamp(1.0, 10_000.0) as u32;
    emitter.burst = count.unwrap_or(if emitter.burst == 0 {
        second
    } else {
        emitter.burst
    });
    emitter.one_shot = true;
    emitter.rate = 0.0;
    emitter.emitting = true;
    emitter.max_particles = emitter.max_particles.max(emitter.burst);
    let rotation = direction
        .and_then(Vec3::try_normalize)
        .map_or(Quat::IDENTITY, |d| Quat::from_rotation_arc(Vec3::Y, d));
    let transform = Transform {
        position,
        rotation,
        ..Transform::IDENTITY
    };
    let global = GlobalTransform(transform.to_matrix());
    world.spawn((transform, global, emitter))
}

/// The splash of something hitting water at `speed` m/s (downward) with a
/// footprint `radius` metres across: the `"splash"` preset, scaled. Just
/// past `min_speed` it's a few droplets; a body slamming in at 8 m/s
/// throws a tall crown of them.
#[must_use]
pub fn splash_emitter(speed: f32, radius: f32, min_speed: f32) -> ParticleEmitter {
    let base = ParticleEmitter::preset("splash").unwrap_or_default();
    // 0 at the threshold, 1 at ~6.5 m/s over it (a dive from a pier).
    let power = ((speed - min_speed) / 6.5).clamp(0.0, 1.6);
    let radius = radius.clamp(0.05, 3.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a small count
    let burst = (12.0 + 90.0 * power * radius.sqrt()).clamp(8.0, 260.0) as u32;
    ParticleEmitter {
        burst,
        max_particles: burst,
        shape_size: [radius * 0.8, 0.05, radius * 0.8],
        cone_angle: 22.0 + 10.0 * power,
        speed: [1.2 + 2.6 * power, 2.4 + 5.0 * power],
        lifetime: [0.5 + 0.3 * power, 0.9 + 0.6 * power],
        size: [0.08 + 0.08 * power, 0.2 + 0.18 * power],
        ..base
    }
}

/// Spray blown off a wave crest by `wind` (0..1): fine white mist that
/// drifts downwind (point the burst's up axis downwind and a little up).
#[must_use]
pub fn spray_emitter(wind: f32) -> ParticleEmitter {
    let wind = wind.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a small count
    let burst = (10.0 + 22.0 * wind) as u32;
    ParticleEmitter {
        emitting: true,
        rate: 0.0,
        burst,
        one_shot: true,
        max_particles: burst,
        lifetime: [0.6, 1.3],
        shape: EmitterShape::Box,
        shape_size: [1.2, 0.1, 0.5],
        cone_angle: 20.0,
        speed: [2.0 + 3.0 * wind, 3.5 + 5.0 * wind],
        gravity: 2.5,
        drag: 1.2,
        size: [0.18, 0.7],
        size_spread: 0.5,
        color: [0.94, 0.97, 1.0, 0.7],
        color_end: [0.92, 0.96, 1.0, 0.0],
        fade_in: 0.05,
        fade_out: 0.6,
        lit: true,
        soft: 0.3,
        ..ParticleEmitter::default()
    }
}

/// Makes `count` extra particles from `entity`'s emitter next frame.
/// Returns false when it has no emitter.
pub fn burst(world: &mut SceneWorld, entity: hecs::Entity, count: u32) -> bool {
    if world.get::<&ParticleEmitter>(entity).is_err() {
        return false;
    }
    if let Ok(mut state) = world.get::<&mut ParticleState>(entity) {
        state.burst(count);
        return true;
    }
    let state = ParticleState {
        pending: count,
        rng: entity.to_bits().get().wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
        ..ParticleState::default()
    };
    world.insert_one(entity, state).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(world: &mut SceneWorld, seconds: f32) {
        let dt = 1.0 / 60.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for _ in 0..(seconds / dt).round() as usize {
            simulate_particles(world, dt);
        }
    }

    fn state(world: &SceneWorld, entity: hecs::Entity) -> ParticleState {
        ParticleState::clone(&world.get::<&ParticleState>(entity).unwrap())
    }

    fn place(world: &mut SceneWorld, emitter: ParticleEmitter, at: Vec3) -> hecs::Entity {
        let t = Transform::from_position(at);
        world.spawn((t, GlobalTransform(t.to_matrix()), emitter))
    }

    #[test]
    fn emitters_make_particles_at_their_rate_up_to_the_cap() {
        let mut world = SceneWorld::new();
        let e = place(
            &mut world,
            ParticleEmitter {
                rate: 30.0,
                lifetime: [10.0, 10.0],
                ..ParticleEmitter::default()
            },
            Vec3::ZERO,
        );
        run(&mut world, 1.0);
        let n = state(&world, e).len();
        assert!((28..=31).contains(&n), "{n}");
        world.get::<&mut ParticleEmitter>(e).unwrap().max_particles = 40;
        run(&mut world, 2.0);
        assert_eq!(state(&world, e).len(), 40);
        // Stopped: no new ones; the old ones live out their lives.
        world.get::<&mut ParticleEmitter>(e).unwrap().emitting = false;
        run(&mut world, 1.0);
        assert_eq!(state(&world, e).len(), 40);
    }

    #[test]
    fn particles_die_at_the_end_of_their_lifetime() {
        let mut world = SceneWorld::new();
        let e = place(
            &mut world,
            ParticleEmitter {
                rate: 0.0,
                burst: 50,
                lifetime: [0.5, 1.0],
                ..ParticleEmitter::default()
            },
            Vec3::ZERO,
        );
        run(&mut world, 0.1);
        assert_eq!(state(&world, e).len(), 50, "the burst comes at once");
        run(&mut world, 0.6);
        let left = state(&world, e).len();
        assert!(left > 0 && left < 50, "{left}");
        run(&mut world, 0.5);
        assert!(state(&world, e).is_empty());
    }

    #[test]
    fn gravity_drags_and_cones_shape_the_flight() {
        let mut world = SceneWorld::new();
        let up = ParticleEmitter {
            rate: 0.0,
            burst: 200,
            shape: EmitterShape::Point,
            cone_angle: 10.0,
            speed: [5.0, 5.0],
            lifetime: [10.0, 10.0],
            ..ParticleEmitter::default()
        };
        let rising = place(&mut world, up.clone(), Vec3::ZERO);
        let falling = place(
            &mut world,
            ParticleEmitter {
                gravity: 9.81,
                ..up.clone()
            },
            Vec3::new(100.0, 0.0, 0.0),
        );
        let slowed = place(
            &mut world,
            ParticleEmitter { drag: 3.0, ..up },
            Vec3::new(200.0, 0.0, 0.0),
        );
        run(&mut world, 1.0);
        for p in state(&world, rising).positions() {
            // Straight up within 10° of +Y, 5 m away.
            assert!((p.length() - 5.0).abs() < 0.2, "{p}");
            assert!(p.y / p.length() > 10f32.to_radians().cos() - 0.01, "{p}");
        }
        let mean_y = |e| {
            let s = state(&world, e);
            #[allow(clippy::cast_precision_loss)]
            let n = s.len() as f32;
            s.positions().map(|p| p.y).sum::<f32>() / n
        };
        let (y_rise, y_fall, y_slow) = (mean_y(rising), mean_y(falling), mean_y(slowed));
        // Falling: about 5 - g/2 after a second.
        assert!(
            (y_fall - (y_rise - 4.9)).abs() < 0.2,
            "{y_fall} vs {y_rise}"
        );
        assert!(y_slow < y_rise - 1.5, "drag slows them: {y_slow}");
    }

    #[test]
    fn shapes_place_particles_inside_them_and_spheres_fly_outward() {
        let mut world = SceneWorld::new();
        let still = ParticleEmitter {
            rate: 0.0,
            burst: 300,
            speed: [0.0, 0.0],
            lifetime: [10.0, 10.0],
            ..ParticleEmitter::default()
        };
        let boxed = place(
            &mut world,
            ParticleEmitter {
                shape: EmitterShape::Box,
                shape_size: [2.0, 0.5, 1.0],
                ..still.clone()
            },
            Vec3::new(10.0, 0.0, 0.0),
        );
        let ball = place(
            &mut world,
            ParticleEmitter {
                shape: EmitterShape::Sphere,
                shape_size: [1.5, 0.0, 0.0],
                speed: [1.0, 1.0],
                ..still.clone()
            },
            Vec3::ZERO,
        );
        let disc = place(
            &mut world,
            ParticleEmitter {
                shape: EmitterShape::Cone,
                shape_size: [0.5, 0.0, 0.0],
                ..still
            },
            Vec3::new(-10.0, 0.0, 0.0),
        );
        simulate_particles(&mut world, 0.0);
        let mut spread = Vec3::ZERO;
        for p in state(&world, boxed).positions() {
            let d = (p - Vec3::new(10.0, 0.0, 0.0)).abs();
            assert!(d.x <= 2.0 && d.y <= 0.5 && d.z <= 1.0, "{d}");
            spread = spread.max(d);
        }
        assert!(
            spread.x > 1.8 && spread.y > 0.4 && spread.z > 0.9,
            "{spread}"
        );
        for p in state(&world, disc).positions() {
            let d = p - Vec3::new(-10.0, 0.0, 0.0);
            assert!(d.y.abs() < 1e-5 && d.length() <= 0.5 + 1e-5, "{d}");
        }
        let before: Vec<Vec3> = state(&world, ball).positions().collect();
        assert!(before.iter().all(|p| p.length() <= 1.5 + 1e-4));
        run(&mut world, 0.5);
        let after: Vec<Vec3> = state(&world, ball).positions().collect();
        for (a, b) in before.iter().zip(&after) {
            assert!(b.length() > a.length() + 0.4, "outward: {a} -> {b}");
        }
    }

    #[test]
    fn world_space_particles_stay_behind_and_local_ones_follow() {
        let mut world = SceneWorld::new();
        let still = ParticleEmitter {
            rate: 0.0,
            burst: 10,
            shape: EmitterShape::Point,
            speed: [0.0, 0.0],
            lifetime: [10.0, 10.0],
            ..ParticleEmitter::default()
        };
        let trail = place(&mut world, still.clone(), Vec3::ZERO);
        let carried = place(
            &mut world,
            ParticleEmitter {
                world_space: false,
                ..still
            },
            Vec3::ZERO,
        );
        simulate_particles(&mut world, 0.0);
        let moved = GlobalTransform(Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)));
        for e in [trail, carried] {
            *world.get::<&mut GlobalTransform>(e).unwrap() = moved;
        }
        let mut draw = automata_renderer::DrawList::new();
        record_particles(&world, &mut draw);
        let xs: Vec<f32> = draw
            .particle_batches()
            .iter()
            .map(|b| b.instances[0].position[0])
            .collect();
        assert_eq!(xs.len(), 2);
        assert!(xs.contains(&0.0) && xs.contains(&5.0), "{xs:?}");
    }

    #[test]
    fn particles_grow_change_colour_and_fade_over_their_life() {
        let mut world = SceneWorld::new();
        let emitter = ParticleEmitter {
            rate: 0.0,
            burst: 1,
            lifetime: [1.0, 1.0],
            speed: [0.0, 0.0],
            size: [1.0, 3.0],
            size_spread: 0.0,
            color: [1.0, 0.0, 0.0, 1.0],
            color_end: [0.0, 0.0, 1.0, 1.0],
            fade_in: 0.2,
            fade_out: 0.2,
            stretch: 0.5,
            ..ParticleEmitter::default()
        };
        place(&mut world, emitter, Vec3::ZERO);
        let look = |world: &SceneWorld| {
            let mut draw = automata_renderer::DrawList::new();
            record_particles(world, &mut draw);
            draw.particle_batches()[0].instances[0]
        };
        simulate_particles(&mut world, 0.0);
        let born = look(&world);
        assert!((born.size - 1.0).abs() < 1e-5);
        assert!(born.color[3].abs() < 1e-5, "fades in from nothing");
        run(&mut world, 0.5);
        let mid = look(&world);
        assert!((mid.size - 2.0).abs() < 0.05, "{}", mid.size);
        assert!((mid.color[0] - 0.5).abs() < 0.05 && (mid.color[2] - 0.5).abs() < 0.05);
        assert!((mid.color[3] - 1.0).abs() < 1e-5, "fully visible mid-life");
        run(&mut world, 0.42);
        let old = look(&world);
        assert!(old.color[3] < 0.5, "fading out: {}", old.color[3]);
    }

    #[test]
    fn previews_replay_finished_bursts_instead_of_despawning() {
        let mut world = SceneWorld::new();
        let e = world.spawn((
            ParticleEmitter::preset("sparks").unwrap(),
            GlobalTransform(Mat4::IDENTITY),
        ));
        let mut bursts = 0;
        let mut was_empty = true;
        for _ in 0..(4 * 60) {
            preview_particles(&mut world, 1.0 / 60.0);
            let n = world.get::<&ParticleState>(e).map_or(0, |s| s.len());
            if n > 0 && was_empty {
                bursts += 1;
            }
            was_empty = n == 0;
        }
        assert!(world.contains(e), "the preview keeps the entity");
        assert!(bursts >= 2, "sparks replay: {bursts} bursts in 4 s");
    }

    #[test]
    fn one_shot_bursts_clean_up_after_themselves() {
        let mut world = SceneWorld::new();
        let sparks = ParticleEmitter::preset("sparks").unwrap();
        let e = spawn_burst(
            &mut world,
            sparks,
            Vec3::new(1.0, 2.0, 3.0),
            Some(Vec3::X),
            Some(12),
        );
        simulate_particles(&mut world, 1.0 / 60.0);
        let s = state(&world, e);
        assert_eq!(s.len(), 12);
        // They fly off along +X (the wall's normal), not up.
        run(&mut world, 0.1);
        let mean_x: f32 = state(&world, e).positions().map(|p| p.x - 1.0).sum::<f32>() / 12.0;
        assert!(mean_x > 0.2, "{mean_x}");
        run(&mut world, 1.0);
        assert!(!world.contains(e), "gone once the sparks die");
    }

    #[test]
    fn bursts_add_to_an_emitter_and_need_one() {
        let mut world = SceneWorld::new();
        let e = place(
            &mut world,
            ParticleEmitter {
                rate: 0.0,
                lifetime: [5.0, 5.0],
                ..ParticleEmitter::default()
            },
            Vec3::ZERO,
        );
        assert!(burst(&mut world, e, 7));
        simulate_particles(&mut world, 0.01);
        assert_eq!(state(&world, e).len(), 7);
        assert!(burst(&mut world, e, 3));
        simulate_particles(&mut world, 0.01);
        assert_eq!(state(&world, e).len(), 10);
        let plain = world.spawn((Transform::IDENTITY,));
        assert!(!burst(&mut world, plain, 3));
        // Removing the emitter drops its particles.
        world.remove_one::<ParticleEmitter>(e).unwrap();
        simulate_particles(&mut world, 0.01);
        assert!(world.get::<&ParticleState>(e).is_err());
    }

    #[test]
    fn presets_exist_by_name_and_are_recognised() {
        for name in PRESETS {
            let preset = ParticleEmitter::preset(name).unwrap();
            assert_eq!(preset.preset_name(), Some(name));
            assert_eq!(
                ParticleEmitter::preset(&name.to_uppercase()),
                Some(preset.clone())
            );
            let mut tweaked = preset;
            tweaked.rate += 1.0;
            assert_eq!(tweaked.preset_name(), None);
        }
        assert!(ParticleEmitter::preset("lava").is_none());
        assert_eq!(ParticleEmitter::default().preset_name(), None);
        let fire = ParticleEmitter::preset("fire").unwrap();
        assert_eq!(fire.blend, ParticleBlend::Additive);
        assert!(ParticleEmitter::preset("smoke").unwrap().lit);
    }

    #[test]
    fn fields_set_by_name_clamp_and_reject_nonsense() {
        let mut e = ParticleEmitter::default();
        e.set_field("rate", &FieldValue::Number(-5.0)).unwrap();
        assert!(e.rate.abs() < f32::EPSILON, "no negative rates");
        e.set_field("size", &FieldValue::Number(0.5)).unwrap();
        assert_eq!(e.size, [0.5, 0.5]);
        e.set_field("color", &FieldValue::Numbers(vec![1.0, 0.5, 0.0]))
            .unwrap();
        assert_eq!(e.color, [1.0, 0.5, 0.0, 1.0]);
        e.set_field("blend", &FieldValue::Text("additive".into()))
            .unwrap();
        assert_eq!(e.blend, ParticleBlend::Additive);
        e.set_field("sprite", &FieldValue::Text("PUFF".into()))
            .unwrap();
        assert_eq!(e.sprite, ParticleSprite::Puff);
        e.set_field("burst", &FieldValue::Number(12.4)).unwrap();
        assert_eq!(e.burst, 12);
        e.set_field("lit", &FieldValue::Bool(true)).unwrap();
        assert!(e.lit);
        assert!(e
            .set_field("shape", &FieldValue::Text("donut".into()))
            .is_err());
        assert!(e
            .set_field("rate", &FieldValue::Text("fast".into()))
            .is_err());
        assert!(e.set_field("colour", &FieldValue::Number(1.0)).is_err());
        assert!(e
            .set_field("color", &FieldValue::Numbers(vec![1.0]))
            .is_err());
        assert_eq!(ParticleEmitter::FIELDS.len(), 28);
        for name in ParticleEmitter::FIELDS {
            let probe = [
                FieldValue::Number(1.0),
                FieldValue::Numbers(vec![1.0; 4]),
                FieldValue::Text("point".into()),
                FieldValue::Text("alpha".into()),
                FieldValue::Text("dot".into()),
                FieldValue::Numbers(vec![1.0; 3]),
                FieldValue::Numbers(vec![1.0; 2]),
            ];
            assert!(
                probe
                    .iter()
                    .any(|v| ParticleEmitter::default().set_field(name, v).is_ok()),
                "{name} is settable"
            );
        }
    }

    #[test]
    fn sprite_sheets_play_over_each_particles_life() {
        let e = ParticleEmitter {
            sheet: [2, 2],
            ..ParticleEmitter::default()
        };
        assert_eq!(e.uv(true, 0, 0.0), [0.0, 0.0, 0.5, 0.5]);
        assert_eq!(e.uv(true, 0, 0.3), [0.5, 0.0, 1.0, 0.5]);
        assert_eq!(e.uv(true, 0, 0.6), [0.0, 0.5, 0.5, 1.0]);
        assert_eq!(e.uv(true, 0, 1.0), [0.5, 0.5, 1.0, 1.0]);
        // Without a texture, the built-in sprite (any variant).
        assert_eq!(e.uv(false, 3, 0.5), ParticleSprite::Dot.uv(0));
    }
}
