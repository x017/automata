//! Editable entity properties: one table the inspector, field scrubbing,
//! and typed edits all share.
//!
//! A [`Field`] names one scalar on an entity (e.g. position X, light
//! ambient, collider friction). [`get`] reads it, [`set`] writes it with the
//! field's valid range applied. Booleans are [`Flag`]s (toggled) and
//! enumerations are [`Choice`]s (cycled).

#![forbid(unsafe_code)]

use automata_core::{EntityData, PrimitiveData};
use automata_physics::{BodyType, Shape3D};

/// One editable scalar property of an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    /// Transform position, axis 0..3.
    Position(u8),
    /// Transform rotation in **degrees** (stored as radians), axis 0..3.
    Rotation(u8),
    /// Transform scale, axis 0..3.
    Scale(u8),
    /// Primitive size (cube side / quad side).
    MeshSize,
    /// Mesh color channel 0..4 (RGBA, 0..1).
    MeshColor(u8),
    /// Mesh surface roughness (0 mirror .. 1 chalk).
    MeshRoughness,
    /// Mesh metalness (0 dielectric .. 1 metal).
    MeshMetallic,
    /// Mesh emitted color channel 0..3.
    MeshEmissive(u8),
    /// Multiplier on the emitted color.
    MeshEmissiveStrength,
    /// How much the glow lights the surroundings.
    MeshGlowLight,
    /// Light direction, axis 0..3.
    LightDirection(u8),
    /// Light color channel 0..3 (RGB, 0..1).
    LightColor(u8),
    /// Light ambient fraction (0..1).
    LightAmbient,
    /// How far from the camera sun shadows reach (metres).
    LightShadowDistance,
    /// Sun shadow edge softness (0 sharpest, 1 default).
    LightShadowSoftness,
    /// Compass angle the sunlight comes from, degrees (0 = from +Z).
    LightAzimuth,
    /// Sun height above the horizon, degrees (90 = straight down).
    LightElevation,
    /// The sun's hour on a simple day arc (6 = sunrise, 12 = highest,
    /// 18 = sunset); sets azimuth and elevation together.
    LightTimeOfDay,
    /// Environment sky ambient channel 0..3.
    EnvSky(u8),
    /// Environment ground ambient channel 0..3.
    EnvGround(u8),
    /// Environment background channel 0..3.
    EnvBackground(u8),
    /// Environment fog color channel 0..3.
    EnvFogColor(u8),
    /// Environment exposure multiplier.
    EnvExposure,
    /// Sky color straight up, channel 0..3.
    EnvZenith(u8),
    /// Sun disk diameter, degrees.
    EnvSunSize,
    /// Sun halo brightness.
    EnvSunGlow,
    /// Cloud cover 0..1.
    EnvClouds,
    /// Lens flare strength.
    EnvLensFlare,
    /// Ambient occlusion strength (Beautiful).
    EnvAmbientOcclusion,
    /// Post: bloom strength (High and up).
    EnvBloom,
    /// Post: contrast.
    EnvContrast,
    /// Post: saturation.
    EnvSaturation,
    /// Post: white balance, cool to warm.
    EnvTemperature,
    /// Post: white balance, green to magenta.
    EnvTint,
    /// Post: lift (shadows), channel 0..3.
    EnvLift(u8),
    /// Post: gamma (midtones), channel 0..3.
    EnvGamma(u8),
    /// Post: gain (highlights), channel 0..3.
    EnvGain(u8),
    /// Post: vignette.
    EnvVignette,
    /// Post: film grain.
    EnvGrain,
    /// Post: chromatic aberration.
    EnvChromatic,
    /// Water surface width (X) and length (Z): index 0..2.
    WaterSize(u8),
    /// Water deep color channel 0..3.
    WaterDeep(u8),
    /// Water shallow color channel 0..3.
    WaterShallow(u8),
    /// Water clarity: how deep you can see (m).
    WaterClarity,
    /// Wave height (m).
    WaterWaveHeight,
    /// Wavelength of the main swell (m).
    WaterWaveLength,
    /// Wave speed multiplier.
    WaterWaveSpeed,
    /// Wind direction the waves travel (degrees).
    WaterWaveDirection,
    /// Shoreline foam amount.
    WaterFoam,
    /// Water density for buoyancy (colliders default to 1).
    WaterDensity,
    /// How much the water slows bodies in it.
    WaterDrag,
    /// Caustics strength under the water (0..2).
    WaterCaustics,
    /// Wind over the water (0..1; spray off the crests from 0.5).
    WaterWind,
    /// The scene's wind for grass and spring bones (0..1).
    EnvWind,
    /// Which way the scene's wind blows (degrees around +Y).
    EnvWindDirection,
    /// Grass rectangle width (X) and depth (Z): index 0..2.
    GrassArea(u8),
    /// Grass blades per square metre.
    GrassDensity,
    /// The tallest grass blades (m).
    GrassHeight,
    /// How much shorter some blades are (0..1).
    GrassHeightVariation,
    /// Blade width at the root (m).
    GrassWidth,
    /// Grass colour at the root, channel 0..3.
    GrassBase(u8),
    /// Grass colour at the tip, channel 0..3.
    GrassTip(u8),
    /// How much blades' colours differ (0..1).
    GrassColorVariation,
    /// How far from the camera grass is drawn (m; 0 = any distance).
    GrassDrawDistance,
    /// Which random field grows.
    GrassSeed,
    /// Scatter rectangle width (X) and depth (Z): index 0..2.
    ScatterArea(u8),
    /// Scattered copies per square metre.
    ScatterDensity,
    /// Scattered copies' smallest (0) and largest (1) scale.
    ScatterScale(u8),
    /// Scattered copies follow the slope (0 upright .. 1).
    ScatterAlign,
    /// Where scattered copies go.
    ScatterSeed,
    /// Point light color channel 0..3 (RGB, 0..1).
    PointColor(u8),
    /// Point light brightness.
    PointIntensity,
    /// Where a point light ends (m; 0 = from its brightness).
    PointRange,
    /// A point light's glowing source size (m).
    PointRadius,
    /// Spotlight color channel 0..3 (RGB, 0..1).
    SpotColor(u8),
    /// Spotlight brightness on its axis.
    SpotIntensity,
    /// Where a spotlight ends (m; 0 = from its brightness).
    SpotRange,
    /// Spotlight full-strength core (degrees from the axis).
    SpotInner,
    /// Spotlight cone edge (degrees from the axis).
    SpotOuter,
    /// A spotlight's glowing source size (m).
    SpotRadius,
    /// Environment fog density per unit.
    EnvFogDensity,
    /// Distance where the fog starts.
    EnvFogStart,
    /// Height fog: full density below this height (m).
    EnvFogHeight,
    /// Height fog thinning per metre above `EnvFogHeight` (0 = even fog).
    EnvFogFalloff,
    /// How much the fog glows toward the sun.
    EnvFogSun,
    /// God ray strength (Beautiful).
    EnvGodRays,
    /// Depth of field blur strength (Beautiful).
    EnvDepthOfField,
    /// Distance kept sharp (0 = the camera's target).
    EnvFocusDistance,
    /// Distance either side of the focus kept sharp.
    EnvFocusRange,
    /// Toon shading bands (0 = smooth).
    EnvToonBands,
    /// Softness of the toon bands' edges.
    EnvToonSoftness,
    /// The shade's tint, channel 0..3.
    EnvShadowTint(u8),
    /// Rim light along objects' edges.
    EnvRimLight,
    /// Outline width (px on a 720 px screen; 0 = none).
    EnvOutline,
    /// Outline color, channel 0..3.
    EnvOutlineColor(u8),
    /// Pixel size (px on a 720 px screen; 0 = off).
    EnvPixelate,
    /// Palette levels per channel (0 = full colour).
    EnvPalette,
    /// Camera eye position, axis 0..3.
    CameraPosition(u8),
    /// Camera look-at target, axis 0..3.
    CameraTarget(u8),
    /// Camera vertical field of view, degrees.
    CameraFov,
    /// Camera near clip plane.
    CameraNear,
    /// Camera far clip plane.
    CameraFar,
    /// Rigid body gravity multiplier.
    BodyGravityScale,
    /// Rigid body linear damping.
    BodyLinearDamping,
    /// Rigid body angular damping.
    BodyAngularDamping,
    /// Ball/capsule collider radius.
    ColliderRadius,
    /// Box collider full extent, axis 0..3.
    ColliderExtents(u8),
    /// Capsule collider half height.
    ColliderHalfHeight,
    /// Collider density.
    ColliderDensity,
    /// Collider friction.
    ColliderFriction,
    /// Collider restitution (bounciness, 0..1).
    ColliderRestitution,
    /// Character controller: steepest walkable slope, in degrees.
    CharacterSlope,
    /// Joint anchor in the entity's space, axis 0..3.
    JointAnchor(u8),
    /// Hinge / slider axis in the entity's space, axis 0..3.
    JointAxis(u8),
    /// Joint lower limit (degrees for hinges and balls, metres for sliders).
    JointLimitMin,
    /// Joint upper limit.
    JointLimitMax,
    /// Rope / spring length (0 = the starting distance).
    JointLength,
    /// Spring stiffness.
    JointStiffness,
    /// Spring damping.
    JointDamping,
    /// Motor speed (degrees or metres per second).
    JointMotorSpeed,
    /// Motor target (degrees or metres from the start).
    JointMotorTarget,
    /// Most force the motor may use (0 = no limit).
    JointMotorMaxForce,
    /// Newtons that snap the joint (0 = unbreakable).
    JointBreakForce,
    /// Audio source volume (1 = as recorded).
    AudioVolume,
    /// Audio source pitch / speed (1 = as recorded).
    AudioPitch,
    /// Distance at which a 3D audio source falls silent (metres).
    AudioRange,
    /// Animation playback rate (1 = normal; negative plays backwards).
    AnimSpeed,
    /// Particles made per second.
    ParticleRate,
    /// Particles made at once when the emitter starts.
    ParticleBurst,
    /// Seconds the emitter runs (0 = forever).
    ParticleDuration,
    /// Most particles alive at once.
    ParticleMax,
    /// Particle lifetime, shortest (0) / longest (1), seconds.
    ParticleLifetime(u8),
    /// Emitter shape size: radius (0) or box half extents (0..3), metres.
    ParticleShapeSize(u8),
    /// How far from straight up particles may leave (degrees).
    ParticleConeAngle,
    /// Starting speed, slowest (0) / fastest (1), m/s.
    ParticleSpeed(u8),
    /// Downward acceleration (negative rises), m/s².
    ParticleGravity,
    /// How fast particles slow down.
    ParticleDrag,
    /// Particle size at birth (0) / death (1), metres.
    ParticleSize(u8),
    /// Birth color channel 0..4 (RGB 0..1 before the glow, then alpha).
    ParticleColor(u8),
    /// Birth color brightness (1 = plain; above glows).
    ParticleGlow,
    /// Death color channel 0..4 (as for `ParticleColor`).
    ParticleEndColor(u8),
    /// Death color brightness.
    ParticleEndGlow,
    /// Sprite spin, radians per second.
    ParticleSpin,
    /// Streaks particles along their motion (seconds of travel).
    ParticleStretch,
    /// Fades particles where they meet the scene (metres; 0 = hard).
    ParticleSoft,
}

/// A boolean property of an entity (toggled by a checkbox).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag {
    /// Light casts shadows.
    LightShadows,
    /// Objects block a point light.
    PointShadows,
    /// Objects block a spotlight.
    SpotShadows,
    /// Objects block a glowing mesh's light.
    MeshGlowShadows,
    /// Rigid body participates in simulation.
    BodyEnabled,
    /// Collider is a trigger (sensor).
    ColliderSensor,
    /// Collider collides.
    ColliderEnabled,
    /// Environment distance fog on/off.
    EnvFog,
    /// Environment sky dome with a sun on/off.
    EnvSkyDome,
    /// The sky painted: banded, flat clouds and sun.
    EnvPaintedSky,
    /// Water reaches the horizon.
    WaterEndless,
    /// Audio source repeats.
    AudioLoop,
    /// The emitter makes new particles.
    ParticleEmitting,
    /// The emitter's entity goes away once its particles are done.
    ParticleOneShot,
    /// Particles are lit by the scene's lights (smoke) rather than glowing.
    ParticleLit,
    /// Particles stay where they were made when the emitter moves.
    ParticleWorldSpace,
    /// Particles fill a box around the camera (dust in the air).
    ParticleFollowCamera,
    /// Audio source starts when the game starts.
    AudioPlayOnStart,
    /// Audio source fades with distance from the camera.
    AudioSpatial,
    /// The animation's clip starts over when it ends.
    AnimLoop,
    /// The animation's clip plays when the game starts.
    AnimPlayOnStart,
    /// The clip's travel moves the entity (root motion).
    AnimRootMotion,
    /// The feet follow the ground (foot IK).
    AnimFootIk,
    /// The joint holds.
    JointEnabled,
    /// The two joined bodies still bump into each other.
    JointCollide,
    /// The joint has limits.
    JointLimits,
    /// The joint has a motor.
    JointMotor,
    /// The motor pulls to a target (rather than just turning).
    JointMotorTarget,
}

/// An enumerated property of an entity (cycled by a dropdown-style button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Choice {
    /// Cube → Sphere → Quad primitive.
    MeshPrimitive,
    /// A ready-made surface: Default, Matte, Glossy, Metal, ...
    MeshSurface,
    /// Dynamic → Static → Kinematic (velocity) → Kinematic (position).
    BodyType,
    /// Box → Sphere → Capsule collider shape.
    ColliderShape,
    /// Environment tone mapping curve.
    Tonemapping,
    /// The Environment's style (Realistic, Semi-realistic …; Custom once changed).
    EnvStyle,
    /// A built-in particle look (Fire, Smoke …; Custom once changed).
    ParticlePreset,
    /// Where particles start: Point, Sphere, Box, Cone.
    ParticleShape,
    /// How particles mix with what's behind: Alpha or Additive (glowing).
    ParticleBlend,
    /// The built-in particle picture: Dot, Spark, Star, Flame, Puff.
    ParticleSprite,
    /// Hinge → Ball → Slider → Fixed → Rope → Spring.
    JointKind,
}

fn axis(i: u8) -> usize {
    usize::from(i.min(2))
}

/// Reads a field's current value, or `None` if the entity lacks it.
#[must_use]
pub fn get(data: &EntityData, field: Field) -> Option<f32> {
    use Field as F;
    Some(match field {
        F::Position(i) => data.transform?.position[axis(i)],
        F::Rotation(i) => data.transform?.rotation_euler[axis(i)].to_degrees(),
        F::Scale(i) => data.transform?.scale[axis(i)],
        F::MeshSize => match data.mesh_visual.as_ref()?.primitive {
            PrimitiveData::Cube { size }
            | PrimitiveData::Quad { size }
            | PrimitiveData::Sphere { size } => size,
            PrimitiveData::Custom { .. } | PrimitiveData::Model { .. } => return None,
        },
        F::MeshColor(i) => data.mesh_visual.as_ref()?.color[usize::from(i.min(3))],
        F::MeshRoughness => {
            let surface = data.mesh_visual.as_ref()?.surface;
            surface
                .roughness
                .unwrap_or(automata_renderer::DEFAULT_ROUGHNESS)
        }
        F::MeshMetallic => data.mesh_visual.as_ref()?.surface.metallic.unwrap_or(0.0),
        F::MeshEmissive(i) => data.mesh_visual.as_ref()?.surface.emissive[axis(i)],
        F::MeshEmissiveStrength => data.mesh_visual.as_ref()?.surface.emissive_strength,
        F::MeshGlowLight => data.mesh_visual.as_ref()?.surface.glow_light,
        F::LightDirection(i) => data.directional_light?.direction.to_array()[axis(i)],
        F::LightColor(i) => data.directional_light?.color[axis(i)],
        F::LightAmbient => data.directional_light?.ambient,
        F::LightShadowDistance => data.directional_light?.shadow_distance,
        F::LightShadowSoftness => data.directional_light?.shadow_softness,
        F::LightAzimuth => sun_angles(data.directional_light?.direction).0,
        F::LightElevation => sun_angles(data.directional_light?.direction).1,
        F::LightTimeOfDay => time_of_day(sun_angles(data.directional_light?.direction).0),
        F::EnvSky(i) => data.environment?.sky_color[axis(i)],
        F::EnvGround(i) => data.environment?.ground_color[axis(i)],
        F::EnvBackground(i) => data.environment?.background[axis(i)],
        F::EnvFogColor(i) => data.environment?.fog_color[axis(i)],
        F::EnvExposure => data.environment?.exposure,
        F::EnvZenith(i) => data.environment?.zenith_color[axis(i)],
        F::EnvSunSize => data.environment?.sun_size,
        F::EnvSunGlow => data.environment?.sun_glow,
        F::EnvClouds => data.environment?.clouds,
        F::EnvLensFlare => data.environment?.lens_flare,
        F::EnvAmbientOcclusion => data.environment?.ambient_occlusion,
        F::EnvBloom => data.environment?.post.bloom,
        F::EnvContrast => data.environment?.post.contrast,
        F::EnvSaturation => data.environment?.post.saturation,
        F::EnvTemperature => data.environment?.post.temperature,
        F::EnvTint => data.environment?.post.tint,
        F::EnvLift(i) => data.environment?.post.lift[axis(i)],
        F::EnvGamma(i) => data.environment?.post.gamma[axis(i)],
        F::EnvGain(i) => data.environment?.post.gain[axis(i)],
        F::EnvVignette => data.environment?.post.vignette,
        F::EnvGrain => data.environment?.post.grain,
        F::EnvChromatic => data.environment?.post.chromatic_aberration,
        F::EnvFogDensity => data.environment?.fog_density,
        F::EnvFogStart => data.environment?.fog_start,
        F::EnvFogHeight => data.environment?.fog_height,
        F::EnvFogFalloff => data.environment?.fog_falloff,
        F::EnvFogSun => data.environment?.fog_sun,
        F::EnvGodRays => data.environment?.god_rays,
        F::EnvDepthOfField => data.environment?.depth_of_field,
        F::EnvFocusDistance => data.environment?.focus_distance,
        F::EnvFocusRange => data.environment?.focus_range,
        F::EnvToonBands | F::EnvPalette => return env_count(data, field),
        F::EnvToonSoftness => data.environment?.toon_softness,
        F::EnvShadowTint(i) => data.environment?.shadow_tint[axis(i)],
        F::EnvRimLight => data.environment?.rim_light,
        F::EnvOutline => data.environment?.outline,
        F::EnvOutlineColor(i) => data.environment?.outline_color[axis(i)],
        F::EnvPixelate => data.environment?.pixelate,
        F::WaterSize(i) => data.water?.size[usize::from(i.min(1))],
        F::WaterDeep(i) => data.water?.deep_color[axis(i)],
        F::WaterShallow(i) => data.water?.shallow_color[axis(i)],
        F::WaterClarity => data.water?.clarity,
        F::WaterWaveHeight => data.water?.wave_height,
        F::WaterWaveLength => data.water?.wave_length,
        F::WaterWaveSpeed => data.water?.wave_speed,
        F::WaterWaveDirection => data.water?.wave_direction,
        F::WaterFoam => data.water?.foam,
        F::WaterDensity => data.water?.density,
        F::WaterDrag => data.water?.drag,
        F::WaterCaustics => data.water?.caustics,
        F::WaterWind => data.water?.wind,
        F::EnvWind => data.environment?.wind,
        F::EnvWindDirection => data.environment?.wind_direction,
        F::GrassArea(_)
        | F::GrassDensity
        | F::GrassHeight
        | F::GrassHeightVariation
        | F::GrassWidth
        | F::GrassBase(_)
        | F::GrassTip(_)
        | F::GrassColorVariation
        | F::GrassDrawDistance
        | F::GrassSeed => grass_get(data.grass.as_ref()?, field)?,
        F::ScatterArea(_)
        | F::ScatterDensity
        | F::ScatterScale(_)
        | F::ScatterAlign
        | F::ScatterSeed => scatter_get(data.scatter.as_ref()?, field)?,
        F::ParticleRate
        | F::ParticleBurst
        | F::ParticleDuration
        | F::ParticleMax
        | F::ParticleLifetime(_)
        | F::ParticleShapeSize(_)
        | F::ParticleConeAngle
        | F::ParticleSpeed(_)
        | F::ParticleGravity
        | F::ParticleDrag
        | F::ParticleSize(_)
        | F::ParticleColor(_)
        | F::ParticleGlow
        | F::ParticleEndColor(_)
        | F::ParticleEndGlow
        | F::ParticleSpin
        | F::ParticleStretch
        | F::ParticleSoft => particle_get(data.particle_emitter.as_ref()?, field)?,
        F::PointColor(i) => data.point_light?.color[axis(i)],
        F::PointIntensity => data.point_light?.intensity,
        F::PointRange => data.point_light?.range,
        F::PointRadius => data.point_light?.radius,
        F::SpotColor(i) => data.spot_light?.color[axis(i)],
        F::SpotIntensity => data.spot_light?.intensity,
        F::SpotRange => data.spot_light?.range,
        F::SpotInner => data.spot_light?.inner_angle,
        F::SpotOuter => data.spot_light?.outer_angle,
        F::SpotRadius => data.spot_light?.radius,
        F::CameraPosition(i) => data.camera_3d?.position.to_array()[axis(i)],
        F::CameraTarget(i) => data.camera_3d?.target.to_array()[axis(i)],
        F::CameraFov => data.camera_3d?.fov_y_degrees,
        F::CameraNear => data.camera_3d?.near,
        F::CameraFar => data.camera_3d?.far,
        F::BodyGravityScale => data.rigid_body.as_ref()?.gravity_scale,
        F::BodyLinearDamping => data.rigid_body.as_ref()?.linear_damping,
        F::BodyAngularDamping => data.rigid_body.as_ref()?.angular_damping,
        F::ColliderRadius => match data.collider_3d.as_ref()?.shape {
            Shape3D::Ball { radius } | Shape3D::Capsule { radius, .. } => radius,
            _ => return None,
        },
        F::ColliderExtents(i) => match data.collider_3d.as_ref()?.shape {
            Shape3D::Cuboid { extents } => extents.to_array()[axis(i)],
            _ => return None,
        },
        F::ColliderHalfHeight => match data.collider_3d.as_ref()?.shape {
            Shape3D::Capsule { half_height, .. } => half_height,
            _ => return None,
        },
        F::ColliderDensity => data.collider_3d.as_ref()?.density,
        F::ColliderFriction => data.collider_3d.as_ref()?.friction,
        F::ColliderRestitution => data.collider_3d.as_ref()?.restitution,
        F::CharacterSlope => data
            .character_controller
            .as_ref()?
            .max_slope_climb_angle
            .to_degrees(),
        F::AudioVolume => data.audio_source.as_ref()?.volume,
        F::AudioPitch => data.audio_source.as_ref()?.pitch,
        F::AudioRange => data.audio_source.as_ref()?.range,
        F::AnimSpeed => data.animation.as_ref()?.speed,
        F::JointAnchor(_)
        | F::JointAxis(_)
        | F::JointLimitMin
        | F::JointLimitMax
        | F::JointLength
        | F::JointStiffness
        | F::JointDamping
        | F::JointMotorSpeed
        | F::JointMotorTarget
        | F::JointMotorMaxForce
        | F::JointBreakForce => joint_get(data.joint.as_ref()?, field)?,
    })
}

/// Hinges and balls show angles in degrees; sliders metres.
fn joint_unit(j: &automata_physics::Joint) -> f32 {
    if j.kind == automata_physics::JointKind::Slider {
        1.0
    } else {
        std::f32::consts::PI / 180.0
    }
}

fn joint_get(j: &automata_physics::Joint, field: Field) -> Option<f32> {
    use Field as F;
    let unit = joint_unit(j);
    Some(match field {
        F::JointAnchor(i) => j.anchor.to_array()[axis(i)],
        F::JointAxis(i) => j.axis.to_array()[axis(i)],
        F::JointLimitMin => j.limits?[0] / unit,
        F::JointLimitMax => j.limits?[1] / unit,
        F::JointLength => j.length,
        F::JointStiffness => j.stiffness,
        F::JointDamping => j.damping,
        F::JointMotorSpeed => j.motor?.speed / unit,
        F::JointMotorTarget => j.motor?.target? / unit,
        F::JointMotorMaxForce => j.motor?.max_force,
        F::JointBreakForce => j.break_force,
        _ => return None,
    })
}

fn joint_set(j: &mut automata_physics::Joint, field: Field, v: f32) -> bool {
    use Field as F;
    let unit = joint_unit(j);
    match field {
        F::JointAnchor(i) => j.anchor[axis(i)] = v,
        F::JointAxis(i) => j.axis[axis(i)] = v,
        F::JointLimitMin | F::JointLimitMax => {
            let Some([lo, hi]) = &mut j.limits else {
                return false;
            };
            // Dragging one past the other carries it along.
            if field == F::JointLimitMin {
                *lo = v * unit;
                *hi = hi.max(*lo);
            } else {
                *hi = v * unit;
                *lo = lo.min(*hi);
            }
        }
        F::JointLength => j.length = v,
        F::JointStiffness => j.stiffness = v,
        F::JointDamping => j.damping = v,
        F::JointMotorSpeed | F::JointMotorTarget | F::JointMotorMaxForce => {
            let Some(m) = &mut j.motor else {
                return false;
            };
            match field {
                F::JointMotorSpeed => m.speed = v * unit,
                F::JointMotorTarget => match &mut m.target {
                    Some(t) => *t = v * unit,
                    None => return false,
                },
                _ => m.max_force = v,
            }
        }
        F::JointBreakForce => j.break_force = v,
        _ => return false,
    }
    true
}

#[allow(clippy::cast_precision_loss)] // seeds stay far below 2^24
fn grass_get(g: &automata_scene::Grass, field: Field) -> Option<f32> {
    use Field as F;
    Some(match field {
        F::GrassArea(i) => g.area[usize::from(i.min(1))],
        F::GrassDensity => g.density,
        F::GrassHeight => g.height,
        F::GrassHeightVariation => g.height_variation,
        F::GrassWidth => g.width,
        F::GrassBase(i) => g.base_color[axis(i)],
        F::GrassTip(i) => g.tip_color[axis(i)],
        F::GrassColorVariation => g.color_variation,
        F::GrassDrawDistance => g.draw_distance,
        F::GrassSeed => g.seed as f32,
        _ => return None,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
fn grass_set(g: &mut automata_scene::Grass, field: Field, v: f32) {
    use Field as F;
    match field {
        F::GrassArea(i) => g.area[usize::from(i.min(1))] = v,
        F::GrassDensity => g.density = v,
        F::GrassHeight => g.height = v,
        F::GrassHeightVariation => g.height_variation = v,
        F::GrassWidth => g.width = v,
        F::GrassBase(i) => g.base_color[axis(i)] = v,
        F::GrassTip(i) => g.tip_color[axis(i)] = v,
        F::GrassColorVariation => g.color_variation = v,
        F::GrassDrawDistance => g.draw_distance = v,
        _ => g.seed = v.round() as u32,
    }
}

#[allow(clippy::cast_precision_loss)] // seeds stay far below 2^24
fn scatter_get(s: &automata_scene::Scatter, field: Field) -> Option<f32> {
    use Field as F;
    Some(match field {
        F::ScatterArea(i) => s.area[usize::from(i.min(1))],
        F::ScatterDensity => s.density,
        F::ScatterScale(i) => s.scale[usize::from(i.min(1))],
        F::ScatterAlign => s.align,
        F::ScatterSeed => s.seed as f32,
        _ => return None,
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
fn scatter_set(s: &mut automata_scene::Scatter, field: Field, v: f32) {
    use Field as F;
    match field {
        F::ScatterArea(i) => s.area[usize::from(i.min(1))] = v,
        F::ScatterDensity => s.density = v,
        // The smallest stays at most the largest.
        F::ScatterScale(0) => {
            s.scale[0] = v;
            s.scale[1] = s.scale[1].max(v);
        }
        F::ScatterScale(_) => {
            s.scale[1] = v;
            s.scale[0] = s.scale[0].min(v);
        }
        F::ScatterAlign => s.align = v,
        _ => s.seed = v.round() as u32,
    }
}

/// The valid range of a field (values are clamped into it on write).
#[must_use]
pub fn range(field: Field) -> (f32, f32) {
    use Field as F;
    match field {
        F::ParticleColor(_)
        | F::ParticleEndColor(_)
        | F::ParticleStretch
        | F::MeshColor(_)
        | F::MeshRoughness
        | F::MeshMetallic
        | F::MeshEmissive(_)
        | F::LightColor(_)
        | F::PointColor(_)
        | F::SpotColor(_)
        | F::LightAmbient
        | F::ColliderRestitution
        | F::EnvSky(_)
        | F::EnvGround(_)
        | F::EnvBackground(_)
        | F::EnvFogColor(_)
        | F::EnvZenith(_)
        | F::EnvOutlineColor(_)
        | F::EnvToonSoftness
        | F::EnvClouds
        | F::EnvVignette
        | F::EnvGrain
        | F::EnvChromatic
        | F::WaterDeep(_)
        | F::WaterShallow(_)
        | F::WaterFoam
        | F::WaterWind
        | F::EnvWind
        | F::GrassBase(_)
        | F::GrassTip(_)
        | F::GrassColorVariation
        | F::GrassHeightVariation
        | F::ScatterAlign => (0.0, 1.0),
        F::GrassArea(_) | F::ScatterArea(_) => (0.0, automata_scene::MAX_AREA),
        F::GrassHeight => (0.01, 10.0),
        F::GrassWidth => (0.002, 1.0),
        F::GrassSeed
        | F::ScatterSeed
        | F::JointLength
        | F::JointStiffness
        | F::JointDamping
        | F::JointMotorMaxForce
        | F::JointBreakForce => (0.0, 1_000_000.0),
        F::ScatterDensity => (0.0, 50.0),
        F::ScatterScale(_) => (0.01, 100.0),
        F::EnvTemperature | F::EnvTint | F::EnvLift(_) | F::JointAxis(_) => (-1.0, 1.0),
        F::EnvContrast => (0.25, 3.0),
        F::EnvSaturation | F::EnvGain(_) => (0.0, 3.0),
        F::EnvGamma(_) => (0.1, 4.0),
        F::PointRadius | F::SpotRadius => (0.01, 5.0),
        F::SpotInner => (0.0, automata_renderer::SpotCone::MAX_ANGLE),
        F::SpotOuter => (1.0, automata_renderer::SpotCone::MAX_ANGLE),
        F::EnvSunSize => (0.05, 20.0),
        F::WaterSize(_) => (0.1, 100_000.0),
        F::WaterClarity => (0.05, 100.0),
        F::WaterWaveHeight | F::WaterWaveSpeed => (0.0, 20.0),
        F::WaterWaveLength => (0.1, 1_000.0),
        F::ParticleGlow | F::ParticleEndGlow => (1.0, 50.0),
        F::ParticleLifetime(_) => (0.01, 600.0),
        F::ParticleConeAngle => (0.0, 180.0),
        F::ParticleSpeed(_) | F::ParticleGravity | F::ParticleSpin => (-1_000.0, 1_000.0),
        F::ParticleMax => (1.0, 100_000.0),
        F::WaterDensity
        | F::WaterDrag
        | F::GrassDensity
        | F::PointIntensity
        | F::SpotIntensity
        | F::PointRange
        | F::SpotRange => (0.0, 1_000.0),
        F::WaterWaveDirection | F::LightAzimuth | F::EnvWindDirection => (-180.0, 180.0),
        F::LightElevation => (-89.0, 89.0),
        F::LightTimeOfDay => (5.0, 19.0),
        F::LightShadowDistance => (10.0, 1_000.0),
        F::LightShadowSoftness => (0.0, 8.0),
        F::EnvExposure | F::EnvPixelate => (0.0, 16.0),
        F::MeshEmissiveStrength => (0.0, 100.0),
        F::MeshGlowLight | F::EnvFogFalloff => (0.0, 10.0),
        F::EnvFogDensity
        | F::EnvAmbientOcclusion
        | F::WaterCaustics
        | F::EnvGodRays
        | F::EnvShadowTint(_)
        | F::EnvRimLight => (0.0, 2.0),
        F::EnvFogStart | F::EnvFocusDistance | F::EnvFocusRange => (0.0, 100_000.0),
        F::EnvPalette => (0.0, 32.0),
        F::EnvFogHeight => (-10_000.0, 10_000.0),
        F::MeshSize | F::ColliderRadius | F::ColliderExtents(_) | F::ColliderHalfHeight => {
            (0.001, 10_000.0)
        }
        F::CameraFov => (1.0, 179.0),
        F::CharacterSlope => (0.0, 89.0),
        F::AudioVolume
        | F::EnvToonBands
        | F::EnvOutline
        | F::EnvSunGlow
        | F::EnvLensFlare
        | F::EnvBloom
        | F::EnvFogSun
        | F::EnvDepthOfField => (0.0, 4.0),
        F::AudioPitch => (0.05, 4.0),
        F::AnimSpeed => (-10.0, 10.0),
        F::AudioRange => (0.5, 10_000.0),
        F::CameraNear => (0.001, 1_000.0),
        F::CameraFar => (0.01, 1_000_000.0),
        F::BodyLinearDamping
        | F::BodyAngularDamping
        | F::GrassDrawDistance
        | F::ColliderDensity
        | F::ColliderFriction
        | F::ParticleRate
        | F::ParticleBurst
        | F::ParticleDuration
        | F::ParticleShapeSize(_)
        | F::ParticleDrag
        | F::ParticleSize(_)
        | F::ParticleSoft => (0.0, 10_000.0),
        _ => (-1_000_000.0, 1_000_000.0),
    }
}

/// Value change per pixel when scrubbing a field with the mouse.
#[must_use]
pub fn scrub_step(field: Field) -> f32 {
    use Field as F;
    match field {
        F::Rotation(_)
        | F::CameraFov
        | F::CharacterSlope
        | F::LightAzimuth
        | F::LightElevation
        | F::SpotInner
        | F::SpotOuter
        | F::ParticleConeAngle
        | F::ParticleRate
        | F::ParticleBurst
        | F::WaterWaveDirection
        | F::EnvWindDirection
        | F::JointLimitMin
        | F::JointLimitMax
        | F::JointMotorSpeed
        | F::JointMotorTarget => 0.5,
        F::JointBreakForce | F::JointMotorMaxForce | F::JointStiffness => 1.0,
        F::GrassSeed | F::ScatterSeed => 0.05,
        F::ParticleMax => 2.0,
        F::EnvFogDensity | F::GrassWidth => 0.0005,
        F::EnvFogFalloff => 0.002,
        F::EnvLift(_) => 0.001,
        F::ParticleColor(_)
        | F::ParticleEndColor(_)
        | F::ParticleStretch
        | F::ParticleSize(_)
        | F::ParticleShapeSize(_)
        | F::ParticleLifetime(_)
        | F::MeshColor(_)
        | F::MeshRoughness
        | F::MeshMetallic
        | F::MeshEmissive(_)
        | F::LightColor(_)
        | F::PointColor(_)
        | F::SpotColor(_)
        | F::LightAmbient
        | F::ColliderRestitution
        | F::EnvSky(_)
        | F::EnvGround(_)
        | F::EnvBackground(_)
        | F::EnvFogColor(_)
        | F::EnvZenith(_)
        | F::EnvClouds
        | F::EnvSunGlow
        | F::EnvLensFlare
        | F::EnvAmbientOcclusion
        | F::EnvFogSun
        | F::EnvGodRays
        | F::EnvDepthOfField
        | F::EnvToonSoftness
        | F::EnvShadowTint(_)
        | F::EnvRimLight
        | F::EnvOutlineColor(_)
        | F::EnvBloom
        | F::EnvContrast
        | F::EnvSaturation
        | F::EnvTemperature
        | F::EnvTint
        | F::EnvGamma(_)
        | F::EnvGain(_)
        | F::EnvVignette
        | F::EnvGrain
        | F::EnvChromatic
        | F::WaterDeep(_)
        | F::WaterShallow(_)
        | F::WaterFoam
        | F::WaterWind
        | F::EnvWind
        | F::GrassBase(_)
        | F::GrassTip(_)
        | F::GrassColorVariation
        | F::GrassHeightVariation
        | F::GrassHeight
        | F::ScatterAlign
        | F::ScatterDensity
        | F::ScatterScale(_)
        | F::WaterCaustics
        | F::WaterWaveHeight
        | F::AudioVolume
        | F::AudioPitch
        | F::AnimSpeed
        | F::EnvExposure => 0.005,
        F::AudioRange | F::LightShadowDistance | F::EnvToonBands | F::EnvPalette => 0.1,
        F::Scale(_) | F::LightDirection(_) => 0.01,
        _ => 0.02,
    }
}

/// Writes a field (clamped to its range). Returns true if the entity has
/// the field. Non-finite values are rejected.
pub fn set(data: &mut EntityData, field: Field, value: f32) -> bool {
    use Field as F;
    if !value.is_finite() {
        return false;
    }
    let (lo, hi) = range(field);
    let v = value.clamp(lo, hi);
    match field {
        F::Position(i) => match &mut data.transform {
            Some(t) => t.position[axis(i)] = v,
            None => return false,
        },
        F::Rotation(i) => match &mut data.transform {
            Some(t) => t.rotation_euler[axis(i)] = v.to_radians(),
            None => return false,
        },
        F::Scale(i) => match &mut data.transform {
            Some(t) => t.scale[axis(i)] = v,
            None => return false,
        },
        F::MeshSize => match data.mesh_visual.as_mut().map(|m| &mut m.primitive) {
            Some(
                PrimitiveData::Cube { size }
                | PrimitiveData::Quad { size }
                | PrimitiveData::Sphere { size },
            ) => *size = v,
            _ => return false,
        },
        F::MeshColor(i) => match &mut data.mesh_visual {
            Some(m) => m.color[usize::from(i.min(3))] = v,
            None => return false,
        },
        F::MeshRoughness
        | F::MeshMetallic
        | F::MeshEmissive(_)
        | F::MeshEmissiveStrength
        | F::MeshGlowLight => {
            let Some(m) = &mut data.mesh_visual else {
                return false;
            };
            let surface = &mut m.surface;
            match field {
                F::MeshRoughness => surface.roughness = Some(v),
                F::MeshMetallic => surface.metallic = Some(v),
                F::MeshEmissive(i) => surface.emissive[axis(i)] = v,
                F::MeshGlowLight => surface.glow_light = v,
                _ => surface.emissive_strength = v,
            }
        }
        F::LightDirection(i) => match &mut data.directional_light {
            Some(l) => l.direction[axis(i)] = v,
            None => return false,
        },
        F::LightColor(i) => match &mut data.directional_light {
            Some(l) => l.color[axis(i)] = v,
            None => return false,
        },
        F::LightAmbient => match &mut data.directional_light {
            Some(l) => l.ambient = v,
            None => return false,
        },
        F::LightShadowDistance => match &mut data.directional_light {
            Some(l) => l.shadow_distance = v,
            None => return false,
        },
        F::LightShadowSoftness => match &mut data.directional_light {
            Some(l) => l.shadow_softness = v,
            None => return false,
        },
        F::LightAzimuth | F::LightElevation => match &mut data.directional_light {
            Some(l) => {
                let (az, el) = sun_angles(l.direction);
                l.direction = if field == F::LightAzimuth {
                    sun_direction(v, el)
                } else {
                    sun_direction(az, v)
                };
            }
            None => return false,
        },
        F::LightTimeOfDay => match &mut data.directional_light {
            Some(l) => {
                let (az, el) = sun_at_hour(v);
                l.direction = sun_direction(az, el);
            }
            None => return false,
        },
        F::EnvSky(_)
        | F::EnvGround(_)
        | F::EnvBackground(_)
        | F::EnvFogColor(_)
        | F::EnvExposure
        | F::EnvZenith(_)
        | F::EnvSunSize
        | F::EnvSunGlow
        | F::EnvClouds
        | F::EnvLensFlare
        | F::EnvAmbientOcclusion
        | F::EnvBloom
        | F::EnvContrast
        | F::EnvSaturation
        | F::EnvTemperature
        | F::EnvTint
        | F::EnvLift(_)
        | F::EnvGamma(_)
        | F::EnvGain(_)
        | F::EnvVignette
        | F::EnvGrain
        | F::EnvChromatic
        | F::EnvFogDensity
        | F::EnvFogStart
        | F::EnvFogHeight
        | F::EnvFogFalloff
        | F::EnvFogSun
        | F::EnvGodRays
        | F::EnvDepthOfField
        | F::EnvFocusDistance
        | F::EnvFocusRange
        | F::EnvToonSoftness
        | F::EnvShadowTint(_)
        | F::EnvRimLight
        | F::EnvOutline
        | F::EnvOutlineColor(_)
        | F::EnvPixelate => match &mut data.environment {
            Some(env) => match field {
                F::EnvSky(i) => env.sky_color[axis(i)] = v,
                F::EnvGround(i) => env.ground_color[axis(i)] = v,
                F::EnvBackground(i) => env.background[axis(i)] = v,
                F::EnvFogColor(i) => env.fog_color[axis(i)] = v,
                F::EnvExposure => env.exposure = v,
                F::EnvZenith(i) => env.zenith_color[axis(i)] = v,
                F::EnvSunSize => env.sun_size = v,
                F::EnvSunGlow => env.sun_glow = v,
                F::EnvClouds => env.clouds = v,
                F::EnvLensFlare => env.lens_flare = v,
                F::EnvAmbientOcclusion => env.ambient_occlusion = v,
                F::EnvBloom => env.post.bloom = v,
                F::EnvContrast => env.post.contrast = v,
                F::EnvSaturation => env.post.saturation = v,
                F::EnvTemperature => env.post.temperature = v,
                F::EnvTint => env.post.tint = v,
                F::EnvLift(i) => env.post.lift[axis(i)] = v,
                F::EnvGamma(i) => env.post.gamma[axis(i)] = v,
                F::EnvGain(i) => env.post.gain[axis(i)] = v,
                F::EnvVignette => env.post.vignette = v,
                F::EnvGrain => env.post.grain = v,
                F::EnvChromatic => env.post.chromatic_aberration = v,
                F::EnvFogHeight => env.fog_height = v,
                F::EnvFogFalloff => env.fog_falloff = v,
                F::EnvFogSun => env.fog_sun = v,
                F::EnvGodRays => env.god_rays = v,
                F::EnvDepthOfField => env.depth_of_field = v,
                F::EnvFocusDistance => env.focus_distance = v,
                F::EnvFocusRange => env.focus_range = v,
                F::EnvToonSoftness => env.toon_softness = v,
                F::EnvShadowTint(i) => env.shadow_tint[axis(i)] = v,
                F::EnvRimLight => env.rim_light = v,
                F::EnvOutline => env.outline = v,
                F::EnvOutlineColor(i) => env.outline_color[axis(i)] = v,
                F::EnvPixelate => env.pixelate = v,
                F::EnvFogDensity => env.fog_density = v,
                _ => env.fog_start = v,
            },
            None => return false,
        },
        F::EnvToonBands | F::EnvPalette => return set_env_count(data, field, v),
        F::EnvWind | F::EnvWindDirection => match &mut data.environment {
            Some(env) if field == F::EnvWind => env.wind = v,
            Some(env) => env.wind_direction = v,
            None => return false,
        },
        F::GrassArea(_)
        | F::GrassDensity
        | F::GrassHeight
        | F::GrassHeightVariation
        | F::GrassWidth
        | F::GrassBase(_)
        | F::GrassTip(_)
        | F::GrassColorVariation
        | F::GrassDrawDistance
        | F::GrassSeed => match &mut data.grass {
            Some(grass) => grass_set(grass, field, v),
            None => return false,
        },
        F::ScatterArea(_)
        | F::ScatterDensity
        | F::ScatterScale(_)
        | F::ScatterAlign
        | F::ScatterSeed => match &mut data.scatter {
            Some(scatter) => scatter_set(scatter, field, v),
            None => return false,
        },
        F::WaterSize(_)
        | F::WaterDeep(_)
        | F::WaterShallow(_)
        | F::WaterClarity
        | F::WaterWaveHeight
        | F::WaterWaveLength
        | F::WaterWaveSpeed
        | F::WaterWaveDirection
        | F::WaterDensity
        | F::WaterDrag
        | F::WaterCaustics
        | F::WaterWind
        | F::WaterFoam => match &mut data.water {
            Some(water) => match field {
                F::WaterSize(i) => water.size[usize::from(i.min(1))] = v,
                F::WaterDeep(i) => water.deep_color[axis(i)] = v,
                F::WaterShallow(i) => water.shallow_color[axis(i)] = v,
                F::WaterClarity => water.clarity = v,
                F::WaterWaveHeight => water.wave_height = v,
                F::WaterWaveLength => water.wave_length = v,
                F::WaterWaveSpeed => water.wave_speed = v,
                F::WaterWaveDirection => water.wave_direction = v,
                F::WaterDensity => water.density = v,
                F::WaterDrag => water.drag = v,
                F::WaterCaustics => water.caustics = v,
                F::WaterWind => water.wind = v,
                _ => water.foam = v,
            },
            None => return false,
        },
        F::PointColor(_) | F::PointIntensity | F::PointRange | F::PointRadius => {
            match &mut data.point_light {
                Some(l) => match field {
                    F::PointColor(i) => l.color[axis(i)] = v,
                    F::PointIntensity => l.intensity = v,
                    F::PointRange => l.range = v,
                    _ => l.radius = v,
                },
                None => return false,
            }
        }
        F::SpotColor(_)
        | F::SpotIntensity
        | F::SpotRange
        | F::SpotInner
        | F::SpotOuter
        | F::SpotRadius => match &mut data.spot_light {
            Some(l) => match field {
                F::SpotColor(i) => l.color[axis(i)] = v,
                F::SpotIntensity => l.intensity = v,
                F::SpotRange => l.range = v,
                // The core stays inside the edge: widening the core pushes
                // the edge out, narrowing the edge pulls the core in.
                F::SpotInner => {
                    l.inner_angle = v;
                    l.outer_angle = l.outer_angle.max(v);
                }
                F::SpotOuter => {
                    l.outer_angle = v;
                    l.inner_angle = l.inner_angle.min(v);
                }
                _ => l.radius = v,
            },
            None => return false,
        },
        F::ParticleRate
        | F::ParticleBurst
        | F::ParticleDuration
        | F::ParticleMax
        | F::ParticleLifetime(_)
        | F::ParticleShapeSize(_)
        | F::ParticleConeAngle
        | F::ParticleSpeed(_)
        | F::ParticleGravity
        | F::ParticleDrag
        | F::ParticleSize(_)
        | F::ParticleColor(_)
        | F::ParticleGlow
        | F::ParticleEndColor(_)
        | F::ParticleEndGlow
        | F::ParticleSpin
        | F::ParticleStretch
        | F::ParticleSoft => match &mut data.particle_emitter {
            Some(e) => particle_set(e, field, v),
            None => return false,
        },
        F::CameraPosition(i) => match &mut data.camera_3d {
            Some(c) => c.position[axis(i)] = v,
            None => return false,
        },
        F::CameraTarget(i) => match &mut data.camera_3d {
            Some(c) => c.target[axis(i)] = v,
            None => return false,
        },
        F::CameraFov => match &mut data.camera_3d {
            Some(c) => c.fov_y_degrees = v,
            None => return false,
        },
        F::CameraNear => match &mut data.camera_3d {
            Some(c) => c.near = v,
            None => return false,
        },
        F::CameraFar => match &mut data.camera_3d {
            Some(c) => c.far = v,
            None => return false,
        },
        F::BodyGravityScale => match &mut data.rigid_body {
            Some(b) => b.gravity_scale = v,
            None => return false,
        },
        F::BodyLinearDamping => match &mut data.rigid_body {
            Some(b) => b.linear_damping = v,
            None => return false,
        },
        F::BodyAngularDamping => match &mut data.rigid_body {
            Some(b) => b.angular_damping = v,
            None => return false,
        },
        F::ColliderRadius => match data.collider_3d.as_mut().map(|c| &mut c.shape) {
            Some(Shape3D::Ball { radius } | Shape3D::Capsule { radius, .. }) => *radius = v,
            _ => return false,
        },
        F::ColliderExtents(i) => match data.collider_3d.as_mut().map(|c| &mut c.shape) {
            Some(Shape3D::Cuboid { extents }) => extents[axis(i)] = v,
            _ => return false,
        },
        F::ColliderHalfHeight => match data.collider_3d.as_mut().map(|c| &mut c.shape) {
            Some(Shape3D::Capsule { half_height, .. }) => *half_height = v,
            _ => return false,
        },
        F::ColliderDensity => match &mut data.collider_3d {
            Some(c) => c.density = v,
            None => return false,
        },
        F::ColliderFriction => match &mut data.collider_3d {
            Some(c) => c.friction = v,
            None => return false,
        },
        F::ColliderRestitution => match &mut data.collider_3d {
            Some(c) => c.restitution = v,
            None => return false,
        },
        F::CharacterSlope => match &mut data.character_controller {
            Some(c) => c.max_slope_climb_angle = v.to_radians(),
            None => return false,
        },
        F::JointAnchor(_)
        | F::JointAxis(_)
        | F::JointLimitMin
        | F::JointLimitMax
        | F::JointLength
        | F::JointStiffness
        | F::JointDamping
        | F::JointMotorSpeed
        | F::JointMotorTarget
        | F::JointMotorMaxForce
        | F::JointBreakForce => match &mut data.joint {
            Some(j) => return joint_set(j, field, v),
            None => return false,
        },
        F::AudioVolume | F::AudioPitch | F::AudioRange => match &mut data.audio_source {
            Some(a) => match field {
                F::AudioVolume => a.volume = v,
                F::AudioPitch => a.pitch = v,
                _ => a.range = v,
            },
            None => return false,
        },
        F::AnimSpeed => match &mut data.animation {
            Some(a) => a.speed = v,
            None => return false,
        },
    }
    true
}

/// Reads a boolean property, or `None` if the entity lacks it.
#[must_use]
pub fn get_flag(data: &EntityData, flag: Flag) -> Option<bool> {
    Some(match flag {
        Flag::LightShadows => data.directional_light?.shadows,
        Flag::PointShadows => data.point_light?.shadows,
        Flag::SpotShadows => data.spot_light?.shadows,
        Flag::MeshGlowShadows => data.mesh_visual.as_ref()?.surface.glow_shadows,
        Flag::BodyEnabled => data.rigid_body.as_ref()?.enabled,
        Flag::ColliderSensor => data.collider_3d.as_ref()?.sensor,
        Flag::ColliderEnabled => data.collider_3d.as_ref()?.enabled,
        Flag::EnvFog => data.environment?.fog,
        Flag::EnvSkyDome => data.environment?.sky,
        Flag::EnvPaintedSky => data.environment?.painted_sky,
        Flag::WaterEndless => data.water?.endless,
        Flag::AudioLoop => data.audio_source.as_ref()?.looping,
        Flag::ParticleEmitting => data.particle_emitter.as_ref()?.emitting,
        Flag::ParticleOneShot => data.particle_emitter.as_ref()?.one_shot,
        Flag::ParticleLit => data.particle_emitter.as_ref()?.lit,
        Flag::ParticleWorldSpace => data.particle_emitter.as_ref()?.world_space,
        Flag::ParticleFollowCamera => data.particle_emitter.as_ref()?.follow_camera,
        Flag::AudioPlayOnStart => data.audio_source.as_ref()?.play_on_start,
        Flag::AudioSpatial => data.audio_source.as_ref()?.spatial,
        Flag::AnimLoop => data.animation.as_ref()?.looping,
        Flag::AnimPlayOnStart => data.animation.as_ref()?.play_on_start,
        Flag::AnimRootMotion => data.animation.as_ref()?.root_motion,
        Flag::AnimFootIk => data.animation.as_ref()?.foot_ik,
        Flag::JointEnabled => data.joint.as_ref()?.enabled,
        Flag::JointCollide => data.joint.as_ref()?.collide_connected,
        Flag::JointLimits => data.joint.as_ref()?.limits.is_some(),
        Flag::JointMotor => data.joint.as_ref()?.motor.is_some(),
        Flag::JointMotorTarget => data.joint.as_ref()?.motor?.target.is_some(),
    })
}

/// Flips a boolean property. Returns true if the entity has it.
pub fn toggle(data: &mut EntityData, flag: Flag) -> bool {
    match flag {
        Flag::LightShadows => match &mut data.directional_light {
            Some(l) => l.shadows = !l.shadows,
            None => return false,
        },
        Flag::PointShadows => match &mut data.point_light {
            Some(l) => l.shadows = !l.shadows,
            None => return false,
        },
        Flag::SpotShadows => match &mut data.spot_light {
            Some(l) => l.shadows = !l.shadows,
            None => return false,
        },
        Flag::MeshGlowShadows => match &mut data.mesh_visual {
            Some(m) => m.surface.glow_shadows = !m.surface.glow_shadows,
            None => return false,
        },
        Flag::BodyEnabled => match &mut data.rigid_body {
            Some(b) => b.enabled = !b.enabled,
            None => return false,
        },
        Flag::ColliderSensor => match &mut data.collider_3d {
            Some(c) => c.sensor = !c.sensor,
            None => return false,
        },
        Flag::ColliderEnabled => match &mut data.collider_3d {
            Some(c) => c.enabled = !c.enabled,
            None => return false,
        },
        Flag::EnvFog => match &mut data.environment {
            Some(env) => env.fog = !env.fog,
            None => return false,
        },
        Flag::EnvSkyDome => match &mut data.environment {
            Some(env) => env.sky = !env.sky,
            None => return false,
        },
        Flag::EnvPaintedSky => match &mut data.environment {
            Some(env) => env.painted_sky = !env.painted_sky,
            None => return false,
        },
        Flag::WaterEndless => match &mut data.water {
            Some(w) => w.endless = !w.endless,
            None => return false,
        },
        Flag::ParticleEmitting
        | Flag::ParticleOneShot
        | Flag::ParticleLit
        | Flag::ParticleWorldSpace
        | Flag::ParticleFollowCamera => match &mut data.particle_emitter {
            Some(e) => {
                let slot = match flag {
                    Flag::ParticleEmitting => &mut e.emitting,
                    Flag::ParticleOneShot => &mut e.one_shot,
                    Flag::ParticleLit => &mut e.lit,
                    Flag::ParticleFollowCamera => &mut e.follow_camera,
                    _ => &mut e.world_space,
                };
                *slot = !*slot;
            }
            None => return false,
        },
        Flag::AudioLoop | Flag::AudioPlayOnStart | Flag::AudioSpatial => {
            match &mut data.audio_source {
                Some(a) => {
                    let slot = match flag {
                        Flag::AudioLoop => &mut a.looping,
                        Flag::AudioPlayOnStart => &mut a.play_on_start,
                        _ => &mut a.spatial,
                    };
                    *slot = !*slot;
                }
                None => return false,
            }
        }
        Flag::AnimLoop | Flag::AnimPlayOnStart | Flag::AnimRootMotion | Flag::AnimFootIk => {
            match &mut data.animation {
                Some(a) => {
                    let slot = match flag {
                        Flag::AnimLoop => &mut a.looping,
                        Flag::AnimPlayOnStart => &mut a.play_on_start,
                        Flag::AnimFootIk => &mut a.foot_ik,
                        _ => &mut a.root_motion,
                    };
                    *slot = !*slot;
                }
                None => return false,
            }
        }
        Flag::JointEnabled
        | Flag::JointCollide
        | Flag::JointLimits
        | Flag::JointMotor
        | Flag::JointMotorTarget => {
            let Some(j) = &mut data.joint else {
                return false;
            };
            match flag {
                Flag::JointEnabled => j.enabled = !j.enabled,
                Flag::JointCollide => j.collide_connected = !j.collide_connected,
                Flag::JointLimits => {
                    j.limits = match j.limits {
                        Some(_) => None,
                        // ±45° for a hinge or ball; ±0.5 m for a slider.
                        None if j.kind == automata_physics::JointKind::Slider => Some([-0.5, 0.5]),
                        None => Some([-std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4]),
                    };
                }
                Flag::JointMotor => {
                    j.motor = match j.motor {
                        Some(_) => None,
                        None => Some(automata_physics::JointMotor::speed(1.0)),
                    };
                }
                _ => match &mut j.motor {
                    Some(m) => m.target = if m.target.is_some() { None } else { Some(0.0) },
                    None => return false,
                },
            }
        }
    }
    true
}

/// The current option label of an enumerated property.
#[must_use]
pub fn choice_label(data: &EntityData, choice: Choice) -> Option<&'static str> {
    Some(match choice {
        Choice::MeshPrimitive => match data.mesh_visual.as_ref()?.primitive {
            PrimitiveData::Cube { .. } => "Cube",
            PrimitiveData::Quad { .. } => "Quad",
            PrimitiveData::Sphere { .. } => "Sphere",
            PrimitiveData::Custom { .. } => "Custom",
            PrimitiveData::Model { .. } => "Model",
        },
        Choice::MeshSurface => {
            let m = data.mesh_visual.as_ref()?;
            SURFACE_PRESETS
                .iter()
                // (Whether the glow casts shadows is its own checkbox.)
                .find(|(_, make)| {
                    automata_scene::Surface {
                        glow_shadows: m.surface.glow_shadows,
                        ..make(m.color)
                    } == m.surface
                })
                .map_or(CUSTOM_SURFACE, |(label, _)| label)
        }
        Choice::BodyType => match data.rigid_body.as_ref()?.body_type {
            BodyType::Dynamic => "Dynamic",
            BodyType::Static => "Static",
            BodyType::KinematicVelocity => "Kinematic (velocity)",
            BodyType::KinematicPosition => "Kinematic (position)",
        },
        Choice::ColliderShape => data.collider_3d.as_ref()?.shape.label(),
        Choice::Tonemapping => data.environment?.tonemapping.label(),
        Choice::EnvStyle => data.environment?.style_name(),
        Choice::ParticlePreset => data
            .particle_emitter
            .as_ref()?
            .preset_name()
            .unwrap_or(CUSTOM_PARTICLES),
        Choice::ParticleShape => data.particle_emitter.as_ref()?.shape.label(),
        Choice::ParticleBlend => automata_scene::blend_label(data.particle_emitter.as_ref()?.blend),
        Choice::ParticleSprite => data.particle_emitter.as_ref()?.sprite.label(),
        Choice::JointKind => data.joint.as_ref()?.kind.label(),
    })
}

/// Advances an enumerated property to its next option. Returns true if the
/// entity has it (custom meshes don't cycle).
pub fn cycle(data: &mut EntityData, choice: Choice) -> bool {
    match choice {
        Choice::MeshSurface => {
            let Some(label) = choice_label(data, choice) else {
                return false;
            };
            let next = SURFACE_PRESETS
                .iter()
                .position(|(l, _)| *l == label)
                .map_or(0, |i| (i + 1) % SURFACE_PRESETS.len());
            if let Some(m) = &mut data.mesh_visual {
                m.surface = automata_scene::Surface {
                    glow_shadows: m.surface.glow_shadows,
                    ..(SURFACE_PRESETS[next].1)(m.color)
                };
            }
        }
        // Cube → Sphere → Quad → Cube, keeping the size.
        Choice::MeshPrimitive => match data.mesh_visual.as_mut().map(|m| &mut m.primitive) {
            Some(p @ PrimitiveData::Cube { .. }) => {
                let size = match p {
                    PrimitiveData::Cube { size } => *size,
                    _ => 1.0,
                };
                *p = PrimitiveData::Sphere { size };
            }
            Some(p @ PrimitiveData::Sphere { .. }) => {
                let size = match p {
                    PrimitiveData::Sphere { size } => *size,
                    _ => 1.0,
                };
                *p = PrimitiveData::Quad { size };
            }
            Some(p @ PrimitiveData::Quad { .. }) => {
                let size = match p {
                    PrimitiveData::Quad { size } => *size,
                    _ => 1.0,
                };
                *p = PrimitiveData::Cube { size };
            }
            // A model switches back to a plain cube (Model… in the menu
            // picks a file instead of cycling to one).
            Some(p @ PrimitiveData::Model { .. }) => *p = PrimitiveData::Cube { size: 1.0 },
            _ => return false,
        },
        Choice::BodyType => match &mut data.rigid_body {
            Some(b) => {
                b.body_type = match b.body_type {
                    BodyType::Dynamic => BodyType::Static,
                    BodyType::Static => BodyType::KinematicVelocity,
                    BodyType::KinematicVelocity => BodyType::KinematicPosition,
                    BodyType::KinematicPosition => BodyType::Dynamic,
                };
            }
            None => return false,
        },
        Choice::ColliderShape => {
            // Mesh and Convex Hull are offered when there's a model to take
            // the shape from: the entity's own, or the one already in use.
            let model = data
                .collider_3d
                .as_ref()
                .and_then(|c| c.shape.model().map(str::to_string))
                .or_else(|| match data.mesh_visual.as_ref().map(|m| &m.primitive) {
                    Some(PrimitiveData::Model { path }) => Some(path.clone()),
                    _ => None,
                });
            let fitted_box = crate::doc::fitted_box(data);
            let Some(c) = &mut data.collider_3d else {
                return false;
            };
            let to_box = |c: &mut automata_physics::Collider3D| {
                c.shape = fitted_box.shape.clone();
                c.offset = fitted_box.offset;
            };
            match (&c.shape, model) {
                (Shape3D::Cuboid { extents }, _) => {
                    c.shape = Shape3D::Ball {
                        radius: extents.max_element() * 0.5,
                    };
                }
                (&Shape3D::Ball { radius }, _) => {
                    c.shape = Shape3D::Capsule {
                        half_height: radius,
                        radius,
                    };
                }
                (Shape3D::Capsule { .. }, Some(model)) => {
                    c.shape = Shape3D::Mesh { model };
                    c.offset = [0.0; 3];
                }
                (&Shape3D::Capsule { radius, .. }, None) => {
                    c.shape = Shape3D::Cuboid {
                        extents: glam::Vec3::splat(radius * 2.0),
                    };
                }
                (Shape3D::Mesh { .. }, Some(model)) => {
                    c.shape = Shape3D::ConvexHull { model };
                    c.offset = [0.0; 3];
                }
                (Shape3D::Mesh { .. } | Shape3D::ConvexHull { .. }, _) => to_box(c),
            }
        }
        Choice::Tonemapping => match &mut data.environment {
            Some(env) => {
                let all = automata_renderer::Tonemapping::ALL;
                let i = all.iter().position(|t| *t == env.tonemapping).unwrap_or(0);
                env.tonemapping = all[(i + 1) % all.len()];
            }
            None => return false,
        },
        // Custom → Realistic → … → NieR → Realistic.
        Choice::EnvStyle => match &mut data.environment {
            Some(env) => {
                let all = automata_renderer::Environment::STYLES;
                let name = env.style_name();
                let next = all
                    .iter()
                    .position(|(n, _)| *n == name)
                    .map_or(0, |i| (i + 1) % all.len());
                *env = env.with_style(all[next].0).unwrap_or(*env);
            }
            None => return false,
        },
        Choice::ParticlePreset
        | Choice::ParticleShape
        | Choice::ParticleBlend
        | Choice::ParticleSprite => {
            let Some(e) = &mut data.particle_emitter else {
                return false;
            };
            let next = |i: Option<usize>, n: usize| i.map_or(0, |i| (i + 1) % n);
            match choice {
                // Custom → Fire → Smoke → … → Magic → Fire.
                Choice::ParticlePreset => {
                    let all = automata_scene::PARTICLE_PRESETS;
                    let i = e
                        .preset_name()
                        .and_then(|p| all.iter().position(|n| *n == p));
                    if let Some(p) =
                        automata_scene::ParticleEmitter::preset(all[next(i, all.len())])
                    {
                        *e = p;
                    }
                }
                Choice::ParticleShape => {
                    let all = automata_scene::EmitterShape::ALL;
                    e.shape = all[next(all.iter().position(|s| *s == e.shape), all.len())];
                }
                Choice::ParticleBlend => {
                    e.blend = match e.blend {
                        automata_scene::ParticleBlend::Alpha => {
                            automata_scene::ParticleBlend::Additive
                        }
                        automata_scene::ParticleBlend::Additive => {
                            automata_scene::ParticleBlend::Alpha
                        }
                    };
                }
                _ => {
                    let all = automata_scene::ParticleSprite::ALL;
                    e.sprite = all[next(all.iter().position(|s| *s == e.sprite), all.len())];
                }
            }
        }
        Choice::JointKind => {
            use automata_physics::JointKind as K;
            let Some(j) = &mut data.joint else {
                return false;
            };
            let all = K::ALL;
            let i = all.iter().position(|k| *k == j.kind).map_or(0, |i| i + 1);
            let next = all[i % all.len()];
            // Limits and motors are in degrees or metres: they don't carry
            // over between turning and sliding kinds.
            let turning = |k: K| matches!(k, K::Hinge | K::Ball);
            if !next.has_limits() || turning(next) != turning(j.kind) {
                j.limits = None;
            }
            if !next.has_motor() || turning(next) != turning(j.kind) {
                j.motor = None;
            }
            j.kind = next;
        }
    }
    true
}

/// Label of a particle emitter that matches no preset.
const CUSTOM_PARTICLES: &str = "Custom";

/// A particle color's brightness: its brightest channel when that's over
/// 1 (it glows), else 1.
fn glow_of(c: [f32; 4]) -> f32 {
    c[0].max(c[1]).max(c[2]).max(1.0)
}

/// Sets a color's brightness to `glow`: at most 1, the plain color; above,
/// its brightest channel becomes `glow`.
fn set_glow(c: &mut [f32; 4], glow: f32) {
    let old = glow_of(*c);
    let plain = [c[0] / old, c[1] / old, c[2] / old];
    let peak = plain[0].max(plain[1]).max(plain[2]);
    let scale = if glow <= 1.0 || peak <= 1e-6 {
        1.0
    } else {
        glow / peak
    };
    for (slot, p) in c.iter_mut().zip(plain) {
        *slot = p * scale;
    }
}

/// A particle emitter field's value (see [`get`]).
fn particle_get(e: &automata_scene::ParticleEmitter, field: Field) -> Option<f32> {
    use Field as F;
    #[allow(clippy::cast_precision_loss)] // counts shown as numbers
    Some(match field {
        F::ParticleRate => e.rate,
        F::ParticleBurst => e.burst as f32,
        F::ParticleDuration => e.duration,
        F::ParticleMax => e.max_particles as f32,
        F::ParticleLifetime(i) => e.lifetime[usize::from(i.min(1))],
        F::ParticleShapeSize(i) => e.shape_size[axis(i)],
        F::ParticleConeAngle => e.cone_angle,
        F::ParticleSpeed(i) => e.speed[usize::from(i.min(1))],
        F::ParticleGravity => e.gravity,
        F::ParticleDrag => e.drag,
        F::ParticleSize(i) => e.size[usize::from(i.min(1))],
        F::ParticleColor(3) => e.color[3],
        F::ParticleColor(i) => e.color[axis(i)] / glow_of(e.color),
        F::ParticleGlow => glow_of(e.color),
        F::ParticleEndColor(3) => e.color_end[3],
        F::ParticleEndColor(i) => e.color_end[axis(i)] / glow_of(e.color_end),
        F::ParticleEndGlow => glow_of(e.color_end),
        F::ParticleSpin => e.spin,
        F::ParticleStretch => e.stretch,
        F::ParticleSoft => e.soft,
        _ => return None,
    })
}

/// Writes a particle emitter field (`v` already clamped to its range).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // counts
fn particle_set(e: &mut automata_scene::ParticleEmitter, field: Field, v: f32) {
    use Field as F;
    match field {
        F::ParticleRate => e.rate = v,
        F::ParticleBurst => e.burst = v.round() as u32,
        F::ParticleDuration => e.duration = v,
        F::ParticleMax => e.max_particles = v.round() as u32,
        // The shortest stays at most the longest, and the other way round.
        F::ParticleLifetime(0) => {
            e.lifetime[0] = v;
            e.lifetime[1] = e.lifetime[1].max(v);
        }
        F::ParticleLifetime(_) => {
            e.lifetime[1] = v;
            e.lifetime[0] = e.lifetime[0].min(v);
        }
        F::ParticleShapeSize(i) => e.shape_size[axis(i)] = v,
        F::ParticleConeAngle => e.cone_angle = v,
        F::ParticleSpeed(i) => e.speed[usize::from(i.min(1))] = v,
        F::ParticleGravity => e.gravity = v,
        F::ParticleDrag => e.drag = v,
        F::ParticleSize(i) => e.size[usize::from(i.min(1))] = v,
        F::ParticleColor(3) => e.color[3] = v,
        F::ParticleColor(i) => e.color[axis(i)] = v * glow_of(e.color),
        F::ParticleGlow => set_glow(&mut e.color, v),
        F::ParticleEndColor(3) => e.color_end[3] = v,
        F::ParticleEndColor(i) => e.color_end[axis(i)] = v * glow_of(e.color_end),
        F::ParticleEndGlow => set_glow(&mut e.color_end, v),
        F::ParticleSpin => e.spin = v,
        F::ParticleStretch => e.stretch = v,
        _ => e.soft = v,
    }
}

/// Writes a color property (RGB colors ignore alpha). Returns true if the
/// entity has it.
pub fn set_color(data: &mut EntityData, target: crate::ui::ColorTarget, rgba: [f32; 4]) -> bool {
    use crate::ui::ColorTarget;
    let c = rgba.map(|v| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    match target {
        ColorTarget::Attribute(i) => {
            let Some(key) = data.attributes.keys().nth(i).cloned() else {
                return false;
            };
            data.attributes
                .insert(key, automata_core::AttributeValue::Color(c));
            true
        }
        // Particle colors keep their glow: the picked color is scaled by
        // it as a whole (channel by channel would shift the brightest).
        ColorTarget::ParticleColor | ColorTarget::ParticleEndColor => {
            let Some(e) = &mut data.particle_emitter else {
                return false;
            };
            let slot = if target == ColorTarget::ParticleColor {
                &mut e.color
            } else {
                &mut e.color_end
            };
            let glow = glow_of(*slot);
            *slot = [c[0], c[1], c[2], c[3]];
            set_glow(slot, glow);
            true
        }
        _ => {
            let mut ok = true;
            for (i, f) in target.channels().into_iter().enumerate() {
                ok &= set(data, f, c[i]);
            }
            ok
        }
    }
}

/// An Environment count (toon bands, palette levels) as a field value.
fn env_count(data: &EntityData, field: Field) -> Option<f32> {
    let env = data.environment?;
    let n = if field == Field::EnvToonBands {
        env.toon_bands
    } else {
        env.palette
    };
    Some(f32::from(u8::try_from(n).unwrap_or(u8::MAX)))
}

/// Sets an Environment count from a field value (already in range).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // counts
fn set_env_count(data: &mut EntityData, field: Field, v: f32) -> bool {
    let Some(env) = &mut data.environment else {
        return false;
    };
    let n = v.round().max(0.0) as u32;
    if field == Field::EnvToonBands {
        env.toon_bands = n;
    } else {
        env.palette = n;
    }
    true
}

/// Every option of an enumerated property, in cycling order (empty if the
/// entity lacks the property).
#[must_use]
pub fn choice_options(data: &EntityData, choice: Choice) -> Vec<&'static str> {
    if choice == Choice::MeshSurface {
        return SURFACE_PRESETS.iter().map(|(label, _)| *label).collect();
    }
    let mut probe = data.clone();
    let mut out = Vec::new();
    while let Some(label) = choice_label(&probe, choice) {
        if out.contains(&label) || out.len() > 16 {
            break;
        }
        out.push(label);
        if !cycle(&mut probe, choice) {
            break;
        }
    }
    out
}

/// Label of a mesh surface that matches no preset.
const CUSTOM_SURFACE: &str = "Custom";

type SurfacePreset = (&'static str, fn([f32; 4]) -> automata_scene::Surface);

/// The Surface menu: a label and the surface it sets for a mesh of a given
/// color (Glowing glows in that color).
const SURFACE_PRESETS: [SurfacePreset; 7] = [
    ("Default", |_| automata_scene::Surface::default()),
    ("Matte", |_| automata_scene::Surface {
        roughness: Some(0.95),
        metallic: Some(0.0),
        ..automata_scene::Surface::default()
    }),
    ("Glossy", |_| automata_scene::Surface {
        roughness: Some(0.15),
        metallic: Some(0.0),
        ..automata_scene::Surface::default()
    }),
    ("Metal", |_| automata_scene::Surface {
        roughness: Some(0.3),
        metallic: Some(1.0),
        ..automata_scene::Surface::default()
    }),
    ("Brushed Metal", |_| automata_scene::Surface {
        roughness: Some(0.6),
        metallic: Some(1.0),
        ..automata_scene::Surface::default()
    }),
    ("Rubber", |_| automata_scene::Surface {
        roughness: Some(0.8),
        metallic: Some(0.0),
        ..automata_scene::Surface::default()
    }),
    ("Glowing", |color| automata_scene::Surface {
        emissive: [color[0], color[1], color[2]],
        emissive_strength: 2.0,
        ..automata_scene::Surface::default()
    }),
];

/// Sets an enumerated property to the option labelled `label`. Returns
/// true if the option exists.
pub fn set_choice(data: &mut EntityData, choice: Choice, label: &str) -> bool {
    if choice == Choice::MeshSurface {
        let preset = SURFACE_PRESETS.iter().find(|(l, _)| *l == label);
        return match (preset, &mut data.mesh_visual) {
            (Some((_, make)), Some(m)) => {
                m.surface = automata_scene::Surface {
                    glow_shadows: m.surface.glow_shadows,
                    ..make(m.color)
                };
                true
            }
            _ => false,
        };
    }
    for _ in 0..16 {
        match choice_label(data, choice) {
            Some(current) if current == label => return true,
            Some(_) => {
                if !cycle(data, choice) {
                    return false;
                }
            }
            None => return false,
        }
    }
    false
}

/// A light travel direction as `(azimuth, elevation)` in degrees, where
/// the angles describe where the light comes **from** (the sun's position).
#[must_use]
pub fn sun_angles(direction: glam::Vec3) -> (f32, f32) {
    let to_sun = -direction.try_normalize().unwrap_or(glam::Vec3::NEG_Y);
    let elevation = to_sun.y.clamp(-1.0, 1.0).asin().to_degrees();
    let azimuth = if to_sun.x.abs() + to_sun.z.abs() < 1e-6 {
        0.0
    } else {
        to_sun.x.atan2(to_sun.z).to_degrees()
    };
    (azimuth, elevation)
}

/// Highest the sun climbs on the time-of-day arc, degrees.
const NOON_ELEVATION: f32 = 65.0;

/// The sun's azimuth / elevation (degrees) at `hour` on a simple day arc:
/// it rises at 6 from -X, peaks at noon toward +Z, and sets at 18 in +X.
#[must_use]
pub fn sun_at_hour(hour: f32) -> (f32, f32) {
    let from_noon = hour - 12.0;
    let azimuth = (from_noon * 15.0).clamp(-180.0, 180.0);
    let elevation = NOON_ELEVATION * (std::f32::consts::PI * (hour - 6.0) / 12.0).sin();
    (azimuth, elevation)
}

/// The hour (0..24) whose arc position has this azimuth.
#[must_use]
pub fn time_of_day(azimuth: f32) -> f32 {
    (12.0 + azimuth / 15.0).rem_euclid(24.0)
}

/// The light travel direction for a sun at `azimuth` / `elevation` degrees.
#[must_use]
pub fn sun_direction(azimuth: f32, elevation: f32) -> glam::Vec3 {
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    -glam::Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos())
}

/// Formats a value for display: up to 3 decimals, trailing zeros trimmed.
#[must_use]
pub fn format_value(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_core::TransformData;

    fn entity() -> EntityData {
        EntityData {
            name: Some("E".into()),
            transform: Some(TransformData::default()),
            ..Default::default()
        }
    }

    #[test]
    fn position_round_trips() {
        let mut d = entity();
        assert!(set(&mut d, Field::Position(1), 2.5));
        assert_eq!(get(&d, Field::Position(1)), Some(2.5));
    }

    #[test]
    fn rotation_is_edited_in_degrees() {
        let mut d = entity();
        assert!(set(&mut d, Field::Rotation(1), 90.0));
        let rad = d.transform.unwrap().rotation_euler[1];
        assert!((rad - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((get(&d, Field::Rotation(1)).unwrap() - 90.0).abs() < 1e-3);
    }

    #[test]
    fn colors_are_clamped() {
        let mut d = entity();
        crate::doc::ComponentKind::Mesh.add_to(&mut d);
        assert!(set(&mut d, Field::MeshColor(0), 5.0));
        assert_eq!(get(&d, Field::MeshColor(0)), Some(1.0));
    }

    #[test]
    fn missing_component_is_none() {
        let mut d = entity();
        assert_eq!(get(&d, Field::LightAmbient), None);
        assert!(!set(&mut d, Field::LightAmbient, 0.5));
    }

    #[test]
    fn spheres_cycle_with_the_primitives_and_get_ball_colliders() {
        let mut d = entity();
        crate::doc::ComponentKind::Mesh.add_to(&mut d);
        assert_eq!(
            choice_options(&d, Choice::MeshPrimitive),
            ["Cube", "Sphere", "Quad"]
        );
        assert!(set(&mut d, Field::MeshSize, 2.0));
        assert!(set_choice(&mut d, Choice::MeshPrimitive, "Sphere"));
        assert_eq!(
            d.mesh_visual.as_ref().unwrap().primitive,
            PrimitiveData::Sphere { size: 2.0 }
        );
        if let Some(t) = &mut d.transform {
            t.scale = [1.0, 3.0, 1.0];
        }
        assert_eq!(
            crate::doc::fitted_collider(&d).shape,
            automata_physics::Shape3D::Ball { radius: 3.0 }
        );
    }

    #[test]
    fn mesh_surfaces_edit_and_follow_presets() {
        let mut d = entity();
        crate::doc::ComponentKind::Mesh.add_to(&mut d);
        assert_eq!(choice_label(&d, Choice::MeshSurface), Some("Default"));
        assert!(
            (get(&d, Field::MeshRoughness).unwrap() - automata_renderer::DEFAULT_ROUGHNESS).abs()
                < 1e-6
        );
        assert!(set_choice(&mut d, Choice::MeshSurface, "Metal"));
        assert!((get(&d, Field::MeshMetallic).unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(choice_label(&d, Choice::MeshSurface), Some("Metal"));
        assert!(set(&mut d, Field::MeshRoughness, 0.31));
        assert_eq!(choice_label(&d, Choice::MeshSurface), Some("Custom"));
        assert!(!choice_options(&d, Choice::MeshSurface).contains(&"Custom"));
        // Glowing glows in the mesh's own color.
        assert!(set_choice(&mut d, Choice::MeshSurface, "Glowing"));
        let m = d.mesh_visual.as_ref().unwrap();
        assert_eq!(m.surface.emissive, [m.color[0], m.color[1], m.color[2]]);
        assert!((get(&d, Field::MeshGlowLight).unwrap() - 1.0).abs() < 1e-6);
        assert!(set(&mut d, Field::MeshGlowLight, 50.0));
        assert!((get(&d, Field::MeshGlowLight).unwrap() - 10.0).abs() < 1e-6);
        assert_eq!(get_flag(&d, Flag::MeshGlowShadows), Some(true));
        assert!(toggle(&mut d, Flag::MeshGlowShadows));
        assert_eq!(get_flag(&d, Flag::MeshGlowShadows), Some(false));
        assert!(set(&mut d, Field::MeshGlowLight, 1.0));
        assert_eq!(choice_label(&d, Choice::MeshSurface), Some("Glowing"));
        assert!(set(&mut d, Field::MeshEmissiveStrength, 500.0));
        assert!((get(&d, Field::MeshEmissiveStrength).unwrap() - 100.0).abs() < 1e-6);
        assert!(set(&mut d, Field::MeshMetallic, 2.0));
        assert!((get(&d, Field::MeshMetallic).unwrap() - 1.0).abs() < 1e-6);
        assert!(
            cycle(&mut d, Choice::MeshSurface),
            "Custom cycles to a preset"
        );
    }

    #[test]
    fn post_effects_are_editable_fields() {
        let mut d = entity();
        assert_eq!(get(&d, Field::EnvSaturation), None);
        crate::doc::ComponentKind::Environment.add_to(&mut d);
        assert!((get(&d, Field::EnvBloom).unwrap() - 1.0).abs() < 1e-6);
        assert!(set(&mut d, Field::EnvSaturation, 0.4));
        assert!(set(&mut d, Field::EnvGain(1), 1.3));
        assert!(set(&mut d, Field::EnvLift(2), -5.0));
        assert!(set(&mut d, Field::EnvVignette, 3.0));
        let post = d.environment.unwrap().post;
        assert!((post.saturation - 0.4).abs() < 1e-6);
        assert_eq!(post.gain, [1.0, 1.3, 1.0]);
        assert!((post.lift[2] + 1.0).abs() < 1e-6, "clamped to -1");
        assert!((post.vignette - 1.0).abs() < 1e-6, "clamped to 1");
    }

    #[test]
    fn time_of_day_moves_the_sun_across_the_sky() {
        let mut d = entity();
        crate::doc::ComponentKind::Light.add_to(&mut d);
        assert!(set(&mut d, Field::LightTimeOfDay, 12.0));
        assert!((get(&d, Field::LightElevation).unwrap() - NOON_ELEVATION).abs() < 1e-3);
        assert!((get(&d, Field::LightTimeOfDay).unwrap() - 12.0).abs() < 1e-3);
        assert!(set(&mut d, Field::LightTimeOfDay, 18.5));
        assert!(
            get(&d, Field::LightElevation).unwrap() < 0.0,
            "set below the horizon"
        );
        assert!((get(&d, Field::LightTimeOfDay).unwrap() - 18.5).abs() < 1e-3);
        assert!(set(&mut d, Field::LightTimeOfDay, 7.0));
        let (az, el) = sun_angles(d.directional_light.unwrap().direction);
        assert!(
            az < -60.0 && el > 0.0 && el < 25.0,
            "a low morning sun: {az} {el}"
        );
    }

    #[test]
    fn sun_angles_round_trip() {
        for (az, el) in [(0.0, 45.0), (120.0, 30.0), (-90.0, 10.0), (179.0, 80.0)] {
            let (a, e) = sun_angles(sun_direction(az, el));
            assert!(
                (a - az).abs() < 1e-3 && (e - el).abs() < 1e-3,
                "{az} {el} -> {a} {e}"
            );
        }
        let mut d = entity();
        crate::doc::ComponentKind::Light.add_to(&mut d);
        assert!(set(&mut d, Field::LightElevation, 60.0));
        assert!((get(&d, Field::LightElevation).unwrap() - 60.0).abs() < 1e-3);
        assert!(
            d.directional_light.unwrap().direction.y < 0.0,
            "light shines down"
        );
    }

    #[test]
    fn water_fields_edit_the_component() {
        let mut d = EntityData::default();
        assert_eq!(get(&d, Field::WaterWaveHeight), None);
        crate::doc::ComponentKind::Water.add_to(&mut d);
        assert!(set(&mut d, Field::WaterWaveHeight, 0.5));
        assert!(set(&mut d, Field::WaterSize(1), 40.0));
        assert!(
            set(&mut d, Field::WaterShallow(0), 2.0),
            "clamped, not rejected"
        );
        let water = d.water.unwrap();
        assert!((water.wave_height - 0.5).abs() < f32::EPSILON);
        assert!((water.size[1] - 40.0).abs() < f32::EPSILON);
        assert!((water.shallow_color[0] - 1.0).abs() < f32::EPSILON);
        crate::doc::ComponentKind::Water.remove_from(&mut d);
        assert!(!set(&mut d, Field::WaterFoam, 0.5));
    }

    #[test]
    fn environment_fields_edit_the_component() {
        let mut d = entity();
        assert_eq!(get(&d, Field::EnvExposure), None);
        crate::doc::ComponentKind::Environment.add_to(&mut d);
        assert!(set(&mut d, Field::EnvExposure, 2.0));
        assert!(set(&mut d, Field::EnvFogColor(2), 0.25));
        assert!(toggle(&mut d, Flag::EnvFog));
        assert!(cycle(&mut d, Choice::Tonemapping));
        let env = d.environment.unwrap();
        assert!((env.exposure - 2.0).abs() < 1e-6);
        assert!((env.fog_color[2] - 0.25).abs() < 1e-6);
        assert!(
            !env.fog,
            "the daylight default has fog; the toggle turned it off"
        );
        assert_eq!(choice_options(&d, Choice::Tonemapping).len(), 4);
        assert!(set_choice(&mut d, Choice::Tonemapping, "AgX"));
        // Atmosphere: height fog, god rays, depth of field, clamped.
        assert!(set(&mut d, Field::EnvFogFalloff, 0.4));
        assert!(set(&mut d, Field::EnvGodRays, 9.0));
        assert!(set(&mut d, Field::EnvFocusDistance, 12.0));
        let env = d.environment.unwrap();
        assert!((env.fog_falloff - 0.4).abs() < 1e-6);
        assert!((env.god_rays - 2.0).abs() < 1e-6, "god rays top out at 2");
        assert_eq!(get(&d, Field::EnvFocusDistance), Some(12.0));
    }

    #[test]
    fn environment_styles_pick_whole_looks_and_go_custom_when_tweaked() {
        let mut d = entity();
        assert_eq!(choice_label(&d, Choice::EnvStyle), None);
        crate::doc::ComponentKind::Environment.add_to(&mut d);
        let sky = d.environment.unwrap().zenith_color;
        assert_eq!(
            choice_options(&d, Choice::EnvStyle),
            [
                "Custom",
                "Realistic",
                "Semi-realistic",
                "BotW",
                "A Short Hike",
                "NieR"
            ]
        );
        assert!(set_choice(&mut d, Choice::EnvStyle, "A Short Hike"));
        assert_eq!(choice_label(&d, Choice::EnvStyle), Some("A Short Hike"));
        let env = d.environment.unwrap();
        assert!(env.outline > 0.0 && env.pixelate > 1.0 && env.toon_bands == 2);
        assert_eq!(env.zenith_color, sky, "the scene's sky stays");
        // Any knob makes it Custom; counts round and clamp.
        assert!(set(&mut d, Field::EnvPalette, 7.6));
        assert_eq!(get(&d, Field::EnvPalette), Some(8.0));
        assert_eq!(choice_label(&d, Choice::EnvStyle), Some("Custom"));
        assert!(set(&mut d, Field::EnvToonBands, 9.0));
        assert_eq!(d.environment.unwrap().toon_bands, 4);
        assert!(set(&mut d, Field::EnvShadowTint(2), 1.5));
        assert!(set(&mut d, Field::EnvOutlineColor(0), 0.5));
        assert!(set(&mut d, Field::EnvRimLight, 0.4));
        assert!(toggle(&mut d, Flag::EnvPaintedSky));
        let env = d.environment.unwrap();
        assert!((env.shadow_tint[2] - 1.5).abs() < 1e-6 && !env.painted_sky);
        // Cycling from NieR wraps to Realistic.
        assert!(set_choice(&mut d, Choice::EnvStyle, "NieR"));
        assert!(cycle(&mut d, Choice::EnvStyle));
        assert_eq!(choice_label(&d, Choice::EnvStyle), Some("Realistic"));
    }

    #[test]
    fn particle_fields_presets_and_glowing_colors() {
        use crate::ui::ColorTarget;
        let mut d = EntityData::default();
        assert_eq!(get(&d, Field::ParticleRate), None);
        assert!(!set(&mut d, Field::ParticleRate, 5.0));
        crate::doc::ComponentKind::Particles.add_to(&mut d);
        assert_eq!(choice_label(&d, Choice::ParticlePreset), Some("Fire"));
        assert_eq!(
            choice_options(&d, Choice::ParticlePreset),
            vec!["Fire", "Smoke", "Sparks", "Dust", "Splash", "Magic", "Motes"]
        );
        // Any change makes it Custom; picking a preset replaces it whole.
        assert!(set(&mut d, Field::ParticleRate, 80.0));
        assert_eq!(choice_label(&d, Choice::ParticlePreset), Some("Custom"));
        assert!(set_choice(&mut d, Choice::ParticlePreset, "Smoke"));
        let smoke = automata_scene::ParticleEmitter::preset("smoke");
        assert_eq!(d.particle_emitter, smoke);
        assert!(set_choice(&mut d, Choice::ParticleSprite, "Star"));
        assert!(set_choice(&mut d, Choice::ParticleBlend, "Additive"));
        assert!(set_choice(&mut d, Choice::ParticleShape, "Box"));
        assert!(!set_choice(&mut d, Choice::ParticleShape, "Donut"));
        let e = d.particle_emitter.clone().unwrap();
        assert_eq!(e.sprite, automata_scene::ParticleSprite::Star);
        assert_eq!(e.blend, automata_scene::ParticleBlend::Additive);
        assert_eq!(e.shape, automata_scene::EmitterShape::Box);
        assert_eq!(get_flag(&d, Flag::ParticleLit), Some(true));
        assert!(toggle(&mut d, Flag::ParticleLit));
        assert_eq!(get_flag(&d, Flag::ParticleLit), Some(false));
        // Lifetimes stay ordered; counts round.
        assert!(set(&mut d, Field::ParticleLifetime(1), 0.5));
        assert!(get(&d, Field::ParticleLifetime(0)).unwrap() <= 0.5);
        assert!(set(&mut d, Field::ParticleBurst, 7.6));
        assert_eq!(d.particle_emitter.as_ref().unwrap().burst, 8);

        // Fire's start color glows (2.0): the picker shows the plain color
        // and picking keeps the glow.
        assert!(set_choice(&mut d, Choice::ParticlePreset, "Fire"));
        let glow = get(&d, Field::ParticleGlow).unwrap();
        assert!((glow - 2.0).abs() < 1e-5, "{glow}");
        let shown = ColorTarget::ParticleColor.read(&d).unwrap();
        assert!(
            shown[..3].iter().all(|c| (0.0..=1.0).contains(c)),
            "{shown:?}"
        );
        assert!(set_color(
            &mut d,
            ColorTarget::ParticleColor,
            [0.0, 0.5, 1.0, 0.8]
        ));
        let c = d.particle_emitter.as_ref().unwrap().color;
        assert!(
            (c[2] - 2.0).abs() < 1e-5 && (c[1] - 1.0).abs() < 1e-5,
            "{c:?}"
        );
        assert!((c[3] - 0.8).abs() < 1e-6, "alpha is not scaled");
        assert!(set(&mut d, Field::ParticleGlow, 1.0));
        let c = d.particle_emitter.as_ref().unwrap().color;
        assert!(
            (c[2] - 1.0).abs() < 1e-5 && (c[3] - 0.8).abs() < 1e-6,
            "{c:?}"
        );
        assert!(set(&mut d, Field::ParticleGlow, 4.0));
        let c = d.particle_emitter.as_ref().unwrap().color;
        assert!(
            (c[2] - 4.0).abs() < 1e-5 && (c[1] - 2.0).abs() < 1e-5,
            "{c:?}"
        );
    }

    #[test]
    fn placed_light_fields_edit_the_component_and_keep_the_core_inside_the_cone() {
        let mut d = EntityData {
            point_light: Some(automata_scene::PointLight::default()),
            spot_light: Some(automata_scene::SpotLight::new(8.0, 30.0)),
            ..EntityData::default()
        };
        assert!(set(&mut d, Field::PointIntensity, 6.0));
        assert!(set(&mut d, Field::PointColor(2), 2.0), "clamped to 1");
        assert_eq!(get(&d, Field::PointIntensity), Some(6.0));
        assert_eq!(get(&d, Field::PointColor(2)), Some(1.0));
        assert!(set(&mut d, Field::SpotOuter, 120.0));
        assert_eq!(get(&d, Field::SpotOuter), Some(80.0));
        assert!(set(&mut d, Field::SpotOuter, 10.0));
        assert_eq!(get(&d, Field::SpotInner), Some(10.0), "core pulled in");
        assert!(set(&mut d, Field::SpotInner, 20.0));
        assert_eq!(get(&d, Field::SpotOuter), Some(20.0), "edge pushed out");
        assert!(toggle(&mut d, Flag::SpotShadows));
        assert_eq!(get_flag(&d, Flag::SpotShadows), Some(false));
        assert_eq!(get_flag(&d, Flag::PointShadows), Some(true));
        d.point_light = None;
        assert!(!set(&mut d, Field::PointRange, 3.0));
        assert_eq!(get_flag(&d, Flag::PointShadows), None);
    }

    #[test]
    fn body_type_cycles_back_to_dynamic() {
        let mut d = entity();
        crate::doc::ComponentKind::RigidBody.add_to(&mut d);
        for _ in 0..4 {
            assert!(cycle(&mut d, Choice::BodyType));
        }
        assert_eq!(choice_label(&d, Choice::BodyType), Some("Dynamic"));
    }

    #[test]
    fn collider_shape_cycle_exposes_matching_fields() {
        let mut d = entity();
        crate::doc::ComponentKind::Collider.add_to(&mut d);
        assert!(get(&d, Field::ColliderExtents(0)).is_some());
        cycle(&mut d, Choice::ColliderShape);
        assert_eq!(choice_label(&d, Choice::ColliderShape), Some("Sphere"));
        assert!(get(&d, Field::ColliderRadius).is_some());
        assert!(get(&d, Field::ColliderExtents(0)).is_none());
    }

    #[test]
    fn toggles_flip() {
        let mut d = entity();
        crate::doc::ComponentKind::Light.add_to(&mut d);
        let before = get_flag(&d, Flag::LightShadows).unwrap();
        assert!(toggle(&mut d, Flag::LightShadows));
        assert_eq!(get_flag(&d, Flag::LightShadows), Some(!before));
    }

    #[test]
    fn values_format_compactly() {
        assert_eq!(format_value(1.0), "1");
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(-0.0001), "0");
        assert_eq!(format_value(12.3456), "12.346");
    }

    #[test]
    fn joint_fields_show_degrees_for_hinges_and_metres_for_sliders() {
        use automata_physics::{Joint, JointKind};
        let mut d = EntityData::default();
        assert_eq!(get(&d, Field::JointLength), None);
        crate::doc::ComponentKind::Joint.add_to(&mut d);
        assert!(
            d.rigid_body.is_some() && d.collider_3d.is_some(),
            "a joint needs a body"
        );
        assert_eq!(choice_label(&d, Choice::JointKind), Some("Hinge"));
        assert_eq!(get(&d, Field::JointLimitMin), None);
        assert!(!set(&mut d, Field::JointLimitMin, -10.0), "no limits yet");
        assert!(toggle(&mut d, Flag::JointLimits));
        assert!((get(&d, Field::JointLimitMax).unwrap() - 45.0).abs() < 1e-3);
        assert!(set(&mut d, Field::JointLimitMin, 60.0));
        let lim = d.joint.as_ref().unwrap().limits.unwrap();
        assert!((lim[0] - 60f32.to_radians()).abs() < 1e-5 && lim[1] >= lim[0]);
        assert!(toggle(&mut d, Flag::JointMotor) && toggle(&mut d, Flag::JointMotorTarget));
        assert!(set(&mut d, Field::JointMotorTarget, 90.0));
        let target = d.joint.as_ref().unwrap().motor.unwrap().target.unwrap();
        assert!((target - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert_eq!(
            choice_options(&d, Choice::JointKind),
            vec!["Hinge", "Ball", "Slider", "Fixed", "Rope", "Spring"]
        );
        // Ball keeps the angles; Slider drops them (metres now).
        assert!(set_choice(&mut d, Choice::JointKind, "Ball"));
        assert!(d.joint.as_ref().unwrap().limits.is_some());
        assert!(
            d.joint.as_ref().unwrap().motor.is_none(),
            "balls have no motor"
        );
        assert!(set_choice(&mut d, Choice::JointKind, "Slider"));
        assert_eq!(d.joint.as_ref().unwrap().kind, JointKind::Slider);
        assert!(d.joint.as_ref().unwrap().limits.is_none());
        assert!(toggle(&mut d, Flag::JointLimits));
        assert!((get(&d, Field::JointLimitMax).unwrap() - 0.5).abs() < 1e-6);
        assert!(set(&mut d, Field::JointBreakForce, 250.0));
        assert!(toggle(&mut d, Flag::JointCollide) && toggle(&mut d, Flag::JointEnabled));
        let j: &Joint = d.joint.as_ref().unwrap();
        assert!(j.collide_connected && !j.enabled);
        assert!((j.break_force - 250.0).abs() < f32::EPSILON);
    }
}
