//! Scene files: the RON authoring format for worlds.
//!
//! A [`SceneFile`] is plain, versioned data — the on-disk representation a
//! game (or the Oberon editor) saves and loads. Two tolerances keep files
//! durable across engine versions: unknown fields are ignored, and missing
//! fields fall back to defaults, so older engines read newer files and vice
//! versa as far as the overlapping fields allow. [`SCENE_FILE_VERSION`]
//! exists for the day a truly breaking change arrives.
//!
//! Transforms are stored as Euler angles (radians, XYZ order) because scene
//! files are hand-edited; quaternions would round-trip bit-exactly but read
//! as opaque noise. The Euler round-trip drift is far below f32 visual
//! significance.

use automata_renderer::{Camera2D, Camera3D, Environment, MeshData};
use automata_scene::{DirectionalLight, MeshVisual, MeshVisualKind, Name, SceneWorld, Transform};
use glam::EulerRot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The current scene file format version.
pub const SCENE_FILE_VERSION: u32 = 1;

/// Scene file loading and saving failures.
#[derive(Debug, thiserror::Error)]
pub enum SceneFileError {
    /// The RON text is malformed (carries line/column information).
    #[error("invalid scene file: {0}")]
    Parse(#[from] ron::error::SpannedError),
    /// The file was written by a newer engine than this one.
    #[error("scene file version {0} is newer than supported version {SCENE_FILE_VERSION}")]
    UnsupportedVersion(u32),
    /// Serialization failed (not reachable with the current schema's types).
    #[error("scene serialization failed: {0}")]
    Serialize(#[from] ron::Error),
}

/// A serialized scene: a version header plus one record per entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    /// Format version; loading rejects files newer than the engine.
    pub version: u32,
    /// Every entity in the scene.
    #[serde(default)]
    pub entities: Vec<EntityData>,
}

/// One entity's authoring data: an optional name plus at most one of each
/// component. Absent components are simply `None` and omitted from the RON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityData {
    /// The entity's name, used for `ctx.scene` lookups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Local transform (stored as Euler angles for hand-editing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformData>,
    /// Perspective camera.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_3d: Option<Camera3D>,
    /// Orthographic 2D camera.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_2d: Option<Camera2D>,
    /// Directional sun light.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directional_light: Option<DirectionalLight>,
    /// Scene look: ambient, fog, exposure, tone mapping, background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    /// Primitive mesh visual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_visual: Option<MeshVisualData>,
    /// Rigid body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigid_body: Option<automata_physics::RigidBody>,
    /// 3D collider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collider_3d: Option<automata_physics::Collider3D>,
    /// 2D collider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collider_2d: Option<automata_physics::Collider2D>,
    /// Character controller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_controller: Option<automata_physics::CharacterController>,
    /// Character velocity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_velocity: Option<automata_physics::CharacterVelocity>,
    /// Custom named gameplay values (`health: Int(100)`, `team: Text("red")`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, AttributeValue>,
    /// The Python behaviour script attached to the entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<Script>,
    /// A sound the entity plays (see [`crate::audio_source`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_source: Option<crate::AudioSource>,
}

/// A custom attribute value: the small set of types an editor can show and
/// a script can read without schema knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttributeValue {
    /// A true/false flag.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A real number.
    Float(f64),
    /// Free text.
    Text(String),
    /// A 3D vector.
    Vec3([f32; 3]),
    /// An RGBA color (components 0..1).
    Color([f32; 4]),
}

impl AttributeValue {
    /// A short type name ("Bool", "Int", …) for editors and error messages.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "Bool",
            Self::Int(_) => "Int",
            Self::Float(_) => "Float",
            Self::Text(_) => "Text",
            Self::Vec3(_) => "Vec3",
            Self::Color(_) => "Color",
        }
    }
}

/// The custom attributes component: named values attached to an entity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Attributes(pub BTreeMap<String, AttributeValue>);

/// A Python behaviour script attached to an entity (also its ECS component).
///
/// `path` is relative to the scene file's directory (or absolute). The
/// script defines optional `start(entity, ctx)` / `update(entity, ctx)`
/// functions that runners call for the owning entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    /// Path to the `.py` file.
    pub path: String,
}

/// Human-editable transform: position, Euler rotation (XYZ, radians), scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformData {
    /// Translation.
    pub position: [f32; 3],
    /// Euler angles `(x, y, z)` in radians, applied in XYZ order.
    pub rotation_euler: [f32; 3],
    /// Scale per axis.
    pub scale: [f32; 3],
}

impl Default for TransformData {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            rotation_euler: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

impl From<Transform> for TransformData {
    fn from(t: Transform) -> Self {
        Self {
            position: t.position.into(),
            rotation_euler: t.rotation.to_euler(EulerRot::XYZ).into(),
            scale: t.scale.into(),
        }
    }
}

impl From<TransformData> for Transform {
    fn from(d: TransformData) -> Self {
        Self {
            position: d.position.into(),
            rotation: glam::Quat::from_euler(
                EulerRot::XYZ,
                d.rotation_euler[0],
                d.rotation_euler[1],
                d.rotation_euler[2],
            ),
            scale: d.scale.into(),
        }
    }
}

/// The serializable form of a [`MeshVisual`]: a primitive tag plus color,
/// with custom geometry stored verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshVisualData {
    /// Which primitive (or custom geometry) to build.
    pub primitive: PrimitiveData,
    /// Flat base color, RGBA.
    pub color: [f32; 4],
}

/// Primitive geometry descriptors compact enough to hand-edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PrimitiveData {
    /// A cube of `size` units a side.
    Cube {
        /// Side length before scaling.
        size: f32,
    },
    /// A quad of `size` units a side, facing +Z.
    Quad {
        /// Side length before scaling.
        size: f32,
    },
    /// Hand-built geometry, stored as raw vertices and indices.
    Custom {
        /// The full vertex/index data.
        mesh: MeshData,
    },
    /// A model file (`.glb`, `.gltf` or `.obj`), `/`-separated and relative
    /// to the scene file's folder. The visual's color tints it.
    Model {
        /// The model file.
        path: String,
    },
}

impl From<&MeshVisual> for MeshVisualData {
    fn from(visual: &MeshVisual) -> Self {
        let primitive = match &visual.kind {
            MeshVisualKind::Cube { size } => PrimitiveData::Cube { size: *size },
            MeshVisualKind::Quad { size } => PrimitiveData::Quad { size: *size },
            MeshVisualKind::Custom => PrimitiveData::Custom {
                mesh: visual.data.clone(),
            },
            MeshVisualKind::Model { path } => PrimitiveData::Model { path: path.clone() },
        };
        Self {
            primitive,
            color: visual.color,
        }
    }
}

impl From<&MeshVisualData> for MeshVisual {
    fn from(data: &MeshVisualData) -> Self {
        match &data.primitive {
            PrimitiveData::Cube { size } => MeshVisual::cube(*size, data.color),
            PrimitiveData::Quad { size } => MeshVisual::quad(*size, data.color),
            PrimitiveData::Custom { mesh } => MeshVisual::custom(mesh.clone(), data.color),
            PrimitiveData::Model { path } => MeshVisual::model(path, data.color),
        }
    }
}

impl SceneFile {
    /// Captures every entity in a live world.
    #[must_use]
    pub fn from_world(world: &SceneWorld) -> Self {
        let mut entities = Vec::with_capacity(world.len() as usize);
        for eref in world {
            let mut data = EntityData::default();
            if let Some(name) = eref.get::<&Name>() {
                data.name = Some(name.0.clone());
            }
            if let Some(transform) = eref.get::<&Transform>() {
                data.transform = Some(TransformData::from(*transform));
            }
            if let Some(camera) = eref.get::<&Camera3D>() {
                data.camera_3d = Some(*camera);
            }
            if let Some(camera) = eref.get::<&Camera2D>() {
                data.camera_2d = Some(*camera);
            }
            if let Some(light) = eref.get::<&DirectionalLight>() {
                data.directional_light = Some(*light);
            }
            if let Some(env) = eref.get::<&Environment>() {
                data.environment = Some(*env);
            }
            if let Some(visual) = eref.get::<&MeshVisual>() {
                data.mesh_visual = Some(MeshVisualData::from(&*visual));
            }
            if let Some(body) = eref.get::<&automata_physics::RigidBody>() {
                data.rigid_body = Some((*body).clone());
            }
            if let Some(collider) = eref.get::<&automata_physics::Collider3D>() {
                data.collider_3d = Some((*collider).clone());
            }
            if let Some(collider) = eref.get::<&automata_physics::Collider2D>() {
                data.collider_2d = Some((*collider).clone());
            }
            if let Some(controller) = eref.get::<&automata_physics::CharacterController>() {
                data.character_controller = Some((*controller).clone());
            }
            if let Some(velocity) = eref.get::<&automata_physics::CharacterVelocity>() {
                data.character_velocity = Some(*velocity);
            }
            if let Some(attributes) = eref.get::<&Attributes>() {
                data.attributes = attributes.0.clone();
            }
            if let Some(script) = eref.get::<&Script>() {
                data.script = Some((*script).clone());
            }
            if let Some(source) = eref.get::<&crate::AudioSource>() {
                data.audio_source = Some((*source).clone());
            }
            entities.push(data);
        }
        Self {
            version: SCENE_FILE_VERSION,
            entities,
        }
    }

    /// Spawns every recorded entity into `world`, replacing its contents.
    ///
    /// The inverse of [`SceneFile::from_world`]; editors (Oberon) and
    /// tooling use it to materialize a loaded scene file into a live world
    /// for simulation or rendering. Entities get a [`GlobalTransform`] so
    /// the renderer's draw-list recording sees them immediately.
    pub fn apply_to_world(&self, world: &mut SceneWorld) {
        use automata_scene::GlobalTransform;
        world.clear();
        for data in &self.entities {
            let entity = world.spawn(());
            if let Some(name) = &data.name {
                let _ = world.insert_one(entity, Name::new(name.clone()));
            }
            if let Some(transform) = &data.transform {
                let t = Transform::from(*transform);
                let _ = world.insert_one(entity, t);
                let _ = world.insert_one(entity, GlobalTransform(t.to_matrix()));
            }
            if let Some(camera) = &data.camera_3d {
                let _ = world.insert_one(entity, *camera);
            }
            if let Some(camera) = &data.camera_2d {
                let _ = world.insert_one(entity, *camera);
            }
            if let Some(light) = &data.directional_light {
                let _ = world.insert_one(entity, *light);
            }
            if let Some(env) = &data.environment {
                let _ = world.insert_one(entity, *env);
            }
            if let Some(visual) = &data.mesh_visual {
                let _ = world.insert_one(entity, MeshVisual::from(visual));
            }
            if let Some(body) = &data.rigid_body {
                let _ = world.insert_one(entity, body.clone());
            }
            if let Some(collider) = &data.collider_3d {
                let _ = world.insert_one(entity, collider.clone());
            }
            if let Some(collider) = &data.collider_2d {
                let _ = world.insert_one(entity, collider.clone());
            }
            if let Some(controller) = &data.character_controller {
                let _ = world.insert_one(entity, controller.clone());
            }
            if let Some(velocity) = &data.character_velocity {
                let _ = world.insert_one(entity, *velocity);
            }
            if !data.attributes.is_empty() {
                let _ = world.insert_one(entity, Attributes(data.attributes.clone()));
            }
            if let Some(script) = &data.script {
                let _ = world.insert_one(entity, script.clone());
            }
            if let Some(source) = &data.audio_source {
                let _ = world.insert_one(entity, source.clone());
            }
        }
    }

    /// Pretty-printed RON with two-space indentation.
    ///
    /// # Errors
    ///
    /// Returns [`SceneFileError::Serialize`] on failure (not reachable with
    /// the current schema's types).
    pub fn to_ron(&self) -> Result<String, SceneFileError> {
        Ok(ron::ser::to_string_pretty(
            self,
            ron::ser::PrettyConfig::default(),
        )?)
    }

    /// Parses a scene file, rejecting files newer than this engine.
    ///
    /// # Errors
    ///
    /// Returns [`SceneFileError::Parse`] for malformed RON and
    /// [`SceneFileError::UnsupportedVersion`] when `version` is newer than
    /// [`SCENE_FILE_VERSION`].
    pub fn from_ron(text: &str) -> Result<Self, SceneFileError> {
        let file: Self = ron::de::from_str(text)?;
        if file.version > SCENE_FILE_VERSION {
            return Err(SceneFileError::UnsupportedVersion(file.version));
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use automata_physics::{CharacterController, CharacterVelocity, Collider3D, RigidBody};
    use automata_scene::{Name, Scene};

    #[test]
    fn attributes_and_scripts_round_trip_through_ron_and_the_world() {
        let mut data = EntityData {
            name: Some("enemy".into()),
            script: Some(Script {
                path: "scripts/enemy.py".into(),
            }),
            ..EntityData::default()
        };
        data.attributes
            .insert("health".into(), AttributeValue::Int(100));
        data.attributes
            .insert("speed".into(), AttributeValue::Float(4.5));
        data.attributes
            .insert("team".into(), AttributeValue::Text("red".into()));
        data.attributes
            .insert("boss".into(), AttributeValue::Bool(false));
        data.attributes
            .insert("tint".into(), AttributeValue::Color([1.0, 0.5, 0.0, 1.0]));
        data.attributes
            .insert("spawn".into(), AttributeValue::Vec3([1.0, 2.0, 3.0]));
        let file = SceneFile {
            version: SCENE_FILE_VERSION,
            entities: vec![data, EntityData::default()],
        };
        let text = file.to_ron().unwrap();
        assert!(text.contains("health"), "{text}");
        assert!(text.contains("scripts/enemy.py"), "{text}");
        // Entities without attributes/scripts keep the file terse.
        assert_eq!(text.matches("attributes").count(), 1, "{text}");
        let back = SceneFile::from_ron(&text).unwrap();
        assert_eq!(back, file);

        let mut world = SceneWorld::new();
        back.apply_to_world(&mut world);
        let again = SceneFile::from_world(&world);
        let enemy = again
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("enemy"))
            .unwrap();
        assert_eq!(enemy.attributes, file.entities[0].attributes);
        assert_eq!(enemy.script, file.entities[0].script);
    }

    #[test]
    fn audio_sources_round_trip_and_stay_terse() {
        let data = EntityData {
            name: Some("Radio".into()),
            transform: Some(TransformData::default()),
            audio_source: Some(crate::AudioSource {
                looping: true,
                spatial: true,
                ..crate::AudioSource::new("sounds/music.ogg")
            }),
            ..EntityData::default()
        };
        let file = SceneFile {
            version: SCENE_FILE_VERSION,
            entities: vec![data, EntityData::default()],
        };
        let text = file.to_ron().unwrap();
        assert_eq!(text.matches("audio_source").count(), 1, "{text}");
        let back = SceneFile::from_ron(&text).unwrap();
        assert_eq!(back, file);
        // Hand-written files may leave settings out.
        let short = SceneFile::from_ron(
            r#"(version: 1, entities: [(name: Some("Bell"), audio_source: Some((clip: "bell.wav")))])"#,
        )
        .unwrap();
        let source = short.entities[0].audio_source.clone().unwrap();
        assert_eq!(source.clip, "bell.wav");
        assert!((source.volume - 1.0).abs() < 1e-6 && !source.looping);

        let mut world = SceneWorld::new();
        back.apply_to_world(&mut world);
        let again = SceneFile::from_world(&world);
        let radio = again
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("Radio"))
            .unwrap();
        assert_eq!(radio.audio_source, file.entities[0].audio_source);
    }

    #[test]
    fn model_meshes_and_collider_offsets_round_trip() {
        let dir = std::env::temp_dir().join(format!("scene-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("tri.obj");
        std::fs::write(&file_path, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
        let path = file_path.to_string_lossy().replace('\\', "/");
        let data = EntityData {
            name: Some("tree".into()),
            transform: Some(TransformData::default()),
            mesh_visual: Some(MeshVisualData {
                primitive: PrimitiveData::Model { path: path.clone() },
                color: [0.5, 1.0, 1.0, 1.0],
            }),
            collider_3d: Some(
                automata_physics::Collider3D::cuboid(1.0, 1.0, 0.02).offset([0.5, 0.5, 0.0]),
            ),
            ..Default::default()
        };
        let file = SceneFile {
            version: SCENE_FILE_VERSION,
            entities: vec![data],
        };
        let text = file.to_ron().unwrap();
        assert!(text.contains("Model"), "{text}");
        assert!(text.contains("offset"), "{text}");
        assert_eq!(SceneFile::from_ron(&text).unwrap(), file);

        // Into a world: the visual carries the loaded model; and back.
        let mut world = SceneWorld::new();
        file.apply_to_world(&mut world);
        let mut q = world.query::<&automata_scene::MeshVisual>();
        let visual = q.iter().next().cloned().unwrap();
        drop(q);
        assert!(visual.model.is_some(), "the model loads");
        let again = SceneFile::from_world(&world);
        assert_eq!(again.entities[0].mesh_visual, file.entities[0].mesh_visual);
        assert_eq!(again.entities[0].collider_3d, file.entities[0].collider_3d);

        // Colliders without an offset don't write one.
        let plain = SceneFile {
            version: SCENE_FILE_VERSION,
            entities: vec![EntityData {
                collider_3d: Some(automata_physics::Collider3D::ball(0.5)),
                ..Default::default()
            }],
        };
        assert!(!plain.to_ron().unwrap().contains("offset"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn kitchen_sink_world() -> automata_scene::Scene {
        let mut scene = Scene::new();
        let world = scene.world_mut();
        world.spawn((
            Name("player".into()),
            Transform {
                position: glam::Vec3::new(-4.0, 1.4, 0.0),
                rotation: glam::Quat::from_euler(EulerRot::XYZ, 0.0, 0.7, 0.0),
                scale: glam::Vec3::ONE,
            },
            MeshVisual::cube(0.9, [0.95, 0.45, 0.15, 1.0]),
            RigidBody::kinematic_velocity(),
            Collider3D::capsule(0.5, 0.4),
            CharacterController::new().snap_to_ground(0.5),
            CharacterVelocity::default(),
        ));
        world.spawn((
            Name("sun".into()),
            DirectionalLight {
                direction: glam::Vec3::new(-0.45, -1.0, -0.3),
                color: [1.0, 0.96, 0.88],
                ambient: 0.35,
                shadows: true,
            },
        ));
        world.spawn((Camera3D::new(
            glam::Vec3::new(0.0, 2.0, 6.0),
            glam::Vec3::ZERO,
        ),));
        world.spawn((Name("lonely".into()),));
        scene
    }

    #[test]
    fn environment_round_trips_and_tolerates_partial_fields() {
        let mut world = SceneWorld::new();
        world.spawn((automata_renderer::Environment::dusk(),));
        let file = SceneFile::from_world(&world);
        let text = file.to_ron().unwrap();
        let back = SceneFile::from_ron(&text).unwrap();
        assert_eq!(back, file);
        assert_eq!(
            back.entities[0].environment,
            Some(automata_renderer::Environment::dusk())
        );
        // Hand-written files may set only a few fields.
        let partial = SceneFile::from_ron(
            "(version: 1, entities: [(environment: Some((exposure: 2.0, tonemapping: Aces)))])",
        )
        .unwrap();
        let env = partial.entities[0].environment.unwrap();
        assert!((env.exposure - 2.0).abs() < f32::EPSILON);
        assert_eq!(env.tonemapping, automata_renderer::Tonemapping::Aces);
        assert!(!env.fog);
    }

    #[test]
    fn empty_world_round_trips() {
        let scene = Scene::new();
        let file = SceneFile::from_world(scene.world());
        let text = file.to_ron().expect("serialize");
        assert!(text.contains("version: 1"));
        let back = SceneFile::from_ron(&text).expect("parse");
        assert_eq!(back, file);
    }

    #[test]
    fn kitchen_sink_round_trips() {
        let scene = kitchen_sink_world();
        let file = SceneFile::from_world(scene.world());
        assert_eq!(file.entities.len(), 4);
        let text = file.to_ron().expect("serialize");
        let back = SceneFile::from_ron(&text).expect("parse");
        assert_eq!(back, file);

        let player = file
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some("player"))
            .expect("player present");
        let transform = player.transform.expect("transform present");
        assert_eq!(transform.position, [-4.0, 1.4, 0.0]);
        assert!((transform.rotation_euler[1] - 0.7).abs() < 1e-5);
        assert_eq!(
            player.mesh_visual.as_ref().expect("visual").primitive,
            PrimitiveData::Cube { size: 0.9 }
        );
        assert!(player.rigid_body.is_some());
        assert!(player.character_controller.is_some());
    }

    #[test]
    fn ron_output_is_human_readable() {
        let scene = kitchen_sink_world();
        let text = SceneFile::from_world(scene.world())
            .to_ron()
            .expect("serialize");
        assert!(text.contains("name: Some(\"player\")"), "{text}");
        assert!(text.contains("Cube"), "{text}");
        assert!(text.contains("rotation_euler"), "{text}");
    }

    #[test]
    fn from_ron_rejects_garbage() {
        assert!(SceneFile::from_ron("this is not ron").is_err());
    }

    #[test]
    fn from_ron_rejects_newer_versions() {
        let text = "SceneFile(version: 99, entities: [])";
        assert!(SceneFile::from_ron(text).is_err());
    }

    #[test]
    fn missing_fields_default_and_unknown_fields_are_ignored() {
        // A minimal file: entities present, everything else absent.
        let text = "SceneFile(version: 1, entities: [(name: Some(\"crate\"))])";
        let file = SceneFile::from_ron(text).expect("parse");
        assert_eq!(file.entities.len(), 1);
        assert_eq!(file.entities[0].name.as_deref(), Some("crate"));
        assert!(file.entities[0].transform.is_none());
    }
}
