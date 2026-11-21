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
    /// Light direction, axis 0..3.
    LightDirection(u8),
    /// Light color channel 0..3 (RGB, 0..1).
    LightColor(u8),
    /// Light ambient fraction (0..1).
    LightAmbient,
    /// Compass angle the sunlight comes from, degrees (0 = from +Z).
    LightAzimuth,
    /// Sun height above the horizon, degrees (90 = straight down).
    LightElevation,
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
    /// Environment fog density per unit.
    EnvFogDensity,
    /// Distance where the fog starts.
    EnvFogStart,
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
}

/// A boolean property of an entity (toggled by a checkbox).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag {
    /// Light casts shadows.
    LightShadows,
    /// Rigid body participates in simulation.
    BodyEnabled,
    /// Collider is a trigger (sensor).
    ColliderSensor,
    /// Collider collides.
    ColliderEnabled,
    /// Environment distance fog on/off.
    EnvFog,
}

/// An enumerated property of an entity (cycled by a dropdown-style button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Choice {
    /// Cube ↔ Quad primitive.
    MeshPrimitive,
    /// Dynamic → Static → Kinematic (velocity) → Kinematic (position).
    BodyType,
    /// Box → Sphere → Capsule collider shape.
    ColliderShape,
    /// Environment tone mapping curve.
    Tonemapping,
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
            PrimitiveData::Cube { size } | PrimitiveData::Quad { size } => size,
            PrimitiveData::Custom { .. } => return None,
        },
        F::MeshColor(i) => data.mesh_visual.as_ref()?.color[usize::from(i.min(3))],
        F::LightDirection(i) => data.directional_light?.direction.to_array()[axis(i)],
        F::LightColor(i) => data.directional_light?.color[axis(i)],
        F::LightAmbient => data.directional_light?.ambient,
        F::LightAzimuth => sun_angles(data.directional_light?.direction).0,
        F::LightElevation => sun_angles(data.directional_light?.direction).1,
        F::EnvSky(i) => data.environment?.sky_color[axis(i)],
        F::EnvGround(i) => data.environment?.ground_color[axis(i)],
        F::EnvBackground(i) => data.environment?.background[axis(i)],
        F::EnvFogColor(i) => data.environment?.fog_color[axis(i)],
        F::EnvExposure => data.environment?.exposure,
        F::EnvFogDensity => data.environment?.fog_density,
        F::EnvFogStart => data.environment?.fog_start,
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
            Shape3D::Cuboid { .. } => return None,
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
    })
}

/// The valid range of a field (values are clamped into it on write).
#[must_use]
pub fn range(field: Field) -> (f32, f32) {
    use Field as F;
    match field {
        F::MeshColor(_)
        | F::LightColor(_)
        | F::LightAmbient
        | F::ColliderRestitution
        | F::EnvSky(_)
        | F::EnvGround(_)
        | F::EnvBackground(_)
        | F::EnvFogColor(_) => (0.0, 1.0),
        F::LightAzimuth => (-180.0, 180.0),
        F::LightElevation => (-89.0, 89.0),
        F::EnvExposure => (0.0, 16.0),
        F::EnvFogDensity => (0.0, 2.0),
        F::EnvFogStart => (0.0, 100_000.0),
        F::MeshSize | F::ColliderRadius | F::ColliderExtents(_) | F::ColliderHalfHeight => {
            (0.001, 10_000.0)
        }
        F::CameraFov => (1.0, 179.0),
        F::CharacterSlope => (0.0, 89.0),
        F::CameraNear => (0.001, 1_000.0),
        F::CameraFar => (0.01, 1_000_000.0),
        F::BodyLinearDamping | F::BodyAngularDamping | F::ColliderDensity | F::ColliderFriction => {
            (0.0, 10_000.0)
        }
        _ => (-1_000_000.0, 1_000_000.0),
    }
}

/// Value change per pixel when scrubbing a field with the mouse.
#[must_use]
pub fn scrub_step(field: Field) -> f32 {
    use Field as F;
    match field {
        F::Rotation(_) | F::CameraFov | F::CharacterSlope | F::LightAzimuth | F::LightElevation => {
            0.5
        }
        F::EnvFogDensity => 0.0005,
        F::MeshColor(_)
        | F::LightColor(_)
        | F::LightAmbient
        | F::ColliderRestitution
        | F::EnvSky(_)
        | F::EnvGround(_)
        | F::EnvBackground(_)
        | F::EnvFogColor(_)
        | F::EnvExposure => 0.005,
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
            Some(PrimitiveData::Cube { size } | PrimitiveData::Quad { size }) => *size = v,
            _ => return false,
        },
        F::MeshColor(i) => match &mut data.mesh_visual {
            Some(m) => m.color[usize::from(i.min(3))] = v,
            None => return false,
        },
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
        F::EnvSky(_)
        | F::EnvGround(_)
        | F::EnvBackground(_)
        | F::EnvFogColor(_)
        | F::EnvExposure
        | F::EnvFogDensity
        | F::EnvFogStart => match &mut data.environment {
            Some(env) => match field {
                F::EnvSky(i) => env.sky_color[axis(i)] = v,
                F::EnvGround(i) => env.ground_color[axis(i)] = v,
                F::EnvBackground(i) => env.background[axis(i)] = v,
                F::EnvFogColor(i) => env.fog_color[axis(i)] = v,
                F::EnvExposure => env.exposure = v,
                F::EnvFogDensity => env.fog_density = v,
                _ => env.fog_start = v,
            },
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
    }
    true
}

/// Reads a boolean property, or `None` if the entity lacks it.
#[must_use]
pub fn get_flag(data: &EntityData, flag: Flag) -> Option<bool> {
    Some(match flag {
        Flag::LightShadows => data.directional_light?.shadows,
        Flag::BodyEnabled => data.rigid_body.as_ref()?.enabled,
        Flag::ColliderSensor => data.collider_3d.as_ref()?.sensor,
        Flag::ColliderEnabled => data.collider_3d.as_ref()?.enabled,
        Flag::EnvFog => data.environment?.fog,
    })
}

/// Flips a boolean property. Returns true if the entity has it.
pub fn toggle(data: &mut EntityData, flag: Flag) -> bool {
    match flag {
        Flag::LightShadows => match &mut data.directional_light {
            Some(l) => l.shadows = !l.shadows,
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
            PrimitiveData::Custom { .. } => "Custom",
        },
        Choice::BodyType => match data.rigid_body.as_ref()?.body_type {
            BodyType::Dynamic => "Dynamic",
            BodyType::Static => "Static",
            BodyType::KinematicVelocity => "Kinematic (velocity)",
            BodyType::KinematicPosition => "Kinematic (position)",
        },
        Choice::ColliderShape => match data.collider_3d.as_ref()?.shape {
            Shape3D::Cuboid { .. } => "Box",
            Shape3D::Ball { .. } => "Sphere",
            Shape3D::Capsule { .. } => "Capsule",
        },
        Choice::Tonemapping => data.environment?.tonemapping.label(),
    })
}

/// Advances an enumerated property to its next option. Returns true if the
/// entity has it (custom meshes don't cycle).
pub fn cycle(data: &mut EntityData, choice: Choice) -> bool {
    match choice {
        Choice::MeshPrimitive => match data.mesh_visual.as_mut().map(|m| &mut m.primitive) {
            Some(p @ PrimitiveData::Cube { .. }) => {
                let size = match p {
                    PrimitiveData::Cube { size } => *size,
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
        Choice::ColliderShape => match &mut data.collider_3d {
            Some(c) => {
                c.shape = match c.shape {
                    Shape3D::Cuboid { extents } => Shape3D::Ball {
                        radius: extents.max_element() * 0.5,
                    },
                    Shape3D::Ball { radius } => Shape3D::Capsule {
                        half_height: radius,
                        radius,
                    },
                    Shape3D::Capsule { radius, .. } => Shape3D::Cuboid {
                        extents: glam::Vec3::splat(radius * 2.0),
                    },
                };
            }
            None => return false,
        },
        Choice::Tonemapping => match &mut data.environment {
            Some(env) => {
                let all = automata_renderer::Tonemapping::ALL;
                let i = all.iter().position(|t| *t == env.tonemapping).unwrap_or(0);
                env.tonemapping = all[(i + 1) % all.len()];
            }
            None => return false,
        },
    }
    true
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
        _ => {
            let mut ok = true;
            for (i, f) in target.channels().into_iter().enumerate() {
                ok &= set(data, f, c[i]);
            }
            ok
        }
    }
}

/// Every option of an enumerated property, in cycling order (empty if the
/// entity lacks the property).
#[must_use]
pub fn choice_options(data: &EntityData, choice: Choice) -> Vec<&'static str> {
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

/// Sets an enumerated property to the option labelled `label`. Returns
/// true if the option exists.
pub fn set_choice(data: &mut EntityData, choice: Choice, label: &str) -> bool {
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
}
