//! Bridges Python-staged scenes and the RON scene-file schema.
//!
//! The format itself lives in [`automata_core::scene_file`]; this module only
//! converts between its plain data and the engine's staged-component list.

use automata_core::scene_file::{
    EntityData, MeshVisualData, SceneFile, SceneFileError, Script, TransformData,
    SCENE_FILE_VERSION,
};

use crate::components::StagedComponent;
use crate::scene::StagedEntity;

/// Converts one staged entity into its scene-file record.
pub fn staged_to_entity_data(entity: &StagedEntity) -> EntityData {
    let mut data = EntityData {
        name: entity.name.clone(),
        attributes: entity.attributes.clone(),
        script: entity.script.clone().map(|path| Script { path }),
        prefab: entity.prefab.clone(),
        ..EntityData::default()
    };
    for component in &entity.components {
        match component {
            StagedComponent::Transform(t) => data.transform = Some(TransformData::from(*t)),
            StagedComponent::Camera3D(c) => data.camera_3d = Some(*c),
            StagedComponent::Camera2D(c) => data.camera_2d = Some(*c),
            StagedComponent::DirectionalLight(l) => data.directional_light = Some(*l),
            StagedComponent::PointLight(l) => data.point_light = Some(*l),
            StagedComponent::SpotLight(l) => data.spot_light = Some(*l),
            StagedComponent::Environment(e) => data.environment = Some(*e),
            StagedComponent::Water(w) => data.water = Some(*w),
            StagedComponent::AudioSource(a) => data.audio_source = Some(a.clone()),
            StagedComponent::ParticleEmitter(e) => data.particle_emitter = Some(e.clone()),
            StagedComponent::Animation(a) => data.animation = Some(a.clone()),
            StagedComponent::Grass(g) => data.grass = Some(g.clone()),
            StagedComponent::Scatter(s) => data.scatter = Some(s.clone()),
            StagedComponent::Terrain(t) => data.terrain = Some(t.clone()),
            StagedComponent::Joint(j) => data.joint = Some(j.clone()),
            StagedComponent::Attach(a) => data.attach = Some(a.clone()),
            StagedComponent::NavAgent(a) => data.nav_agent = Some(a.clone()),
            StagedComponent::Navigation(n) => data.navigation = Some(n.clone()),
            StagedComponent::NavLink(l) => data.nav_link = Some(l.clone()),
            StagedComponent::Ragdoll(r) => data.ragdoll = Some(r.clone()),
            StagedComponent::MeshVisual(v) => data.mesh_visual = Some(MeshVisualData::from(v)),
            StagedComponent::RigidBody(b) => data.rigid_body = Some(b.clone()),
            StagedComponent::Collider3D(c) => data.collider_3d = Some(c.clone()),
            StagedComponent::Collider2D(c) => data.collider_2d = Some(c.clone()),
            StagedComponent::CharacterController(c) => {
                data.character_controller = Some(c.clone());
            }
            StagedComponent::CharacterVelocity(v) => data.character_velocity = Some(*v),
        }
    }
    data
}

/// Converts one scene-file record back into a staged entity.
pub fn entity_data_to_staged(data: &EntityData) -> StagedEntity {
    let mut components = Vec::new();
    if let Some(t) = &data.transform {
        components.push(StagedComponent::Transform(automata_scene::Transform::from(
            *t,
        )));
    }
    if let Some(c) = &data.camera_3d {
        components.push(StagedComponent::Camera3D(*c));
    }
    if let Some(c) = &data.camera_2d {
        components.push(StagedComponent::Camera2D(*c));
    }
    if let Some(l) = &data.directional_light {
        components.push(StagedComponent::DirectionalLight(*l));
    }
    if let Some(l) = &data.point_light {
        components.push(StagedComponent::PointLight(*l));
    }
    if let Some(l) = &data.spot_light {
        components.push(StagedComponent::SpotLight(*l));
    }
    if let Some(e) = &data.particle_emitter {
        components.push(StagedComponent::ParticleEmitter(e.clone()));
    }
    if let Some(e) = &data.environment {
        components.push(StagedComponent::Environment(*e));
    }
    if let Some(w) = &data.water {
        components.push(StagedComponent::Water(*w));
    }
    if let Some(g) = &data.grass {
        components.push(StagedComponent::Grass(g.clone()));
    }
    if let Some(s) = &data.scatter {
        components.push(StagedComponent::Scatter(s.clone()));
    }
    if let Some(t) = &data.terrain {
        components.push(StagedComponent::Terrain(t.clone()));
    }
    if let Some(j) = &data.joint {
        components.push(StagedComponent::Joint(j.clone()));
    }
    if let Some(a) = &data.attach {
        components.push(StagedComponent::Attach(a.clone()));
    }
    if let Some(a) = &data.nav_agent {
        components.push(StagedComponent::NavAgent(a.clone()));
    }
    if let Some(n) = &data.navigation {
        components.push(StagedComponent::Navigation(n.clone()));
    }
    if let Some(r) = &data.ragdoll {
        components.push(StagedComponent::Ragdoll(r.clone()));
    }
    if let Some(l) = &data.nav_link {
        components.push(StagedComponent::NavLink(l.clone()));
    }
    if let Some(v) = &data.mesh_visual {
        components.push(StagedComponent::MeshVisual(
            automata_scene::MeshVisual::from(v),
        ));
    }
    if let Some(b) = &data.rigid_body {
        components.push(StagedComponent::RigidBody(b.clone()));
    }
    if let Some(c) = &data.collider_3d {
        components.push(StagedComponent::Collider3D(c.clone()));
    }
    if let Some(c) = &data.collider_2d {
        components.push(StagedComponent::Collider2D(c.clone()));
    }
    if let Some(c) = &data.character_controller {
        components.push(StagedComponent::CharacterController(c.clone()));
    }
    if let Some(v) = &data.character_velocity {
        components.push(StagedComponent::CharacterVelocity(*v));
    }
    if let Some(a) = &data.audio_source {
        components.push(StagedComponent::AudioSource(a.clone()));
    }
    if let Some(a) = &data.animation {
        components.push(StagedComponent::Animation(a.clone()));
    }
    StagedEntity {
        name: data.name.clone(),
        components,
        attributes: data.attributes.clone(),
        script: data.script.as_ref().map(|s| s.path.clone()),
        prefab: data.prefab.clone(),
    }
}

/// Serializes a staged scene to RON.
pub fn scene_to_ron(entities: &[StagedEntity]) -> Result<String, SceneFileError> {
    let root = link_root();
    let file = SceneFile {
        version: SCENE_FILE_VERSION,
        entities: entities
            .iter()
            .map(|e| {
                let mut data = staged_to_entity_data(e);
                // A prefab instance lists the fields it changes, as Oberon
                // writes them (kept as stored when the prefab can't be read).
                if let Some(link) = &data.prefab {
                    if let Ok(base) = automata_core::prefab::load_base(link, &root) {
                        let ov = automata_core::prefab::overrides(&base, &data);
                        if let Some(link) = &mut data.prefab {
                            link.overrides = ov;
                        }
                    }
                }
                data
            })
            .collect(),
    };
    file.to_ron()
}

/// Parses a staged scene from RON.
///
/// # Errors
///
/// Propagates [`SceneFileError`] (parse failures and newer file versions).
pub fn scene_from_ron(text: &str) -> Result<Vec<StagedEntity>, SceneFileError> {
    let file = SceneFile::from_ron(text)?;
    Ok(file.entities.iter().map(entity_data_to_staged).collect())
}

/// Parses a scene or prefab file whose prefab instances follow their
/// prefab files (paths relative to `root`, the asset root). Instances whose
/// prefab can't be read keep their stored data, with a warning.
///
/// # Errors
///
/// As [`scene_from_ron`].
pub fn scene_from_file_text(
    text: &str,
    root: &std::path::Path,
) -> Result<Vec<StagedEntity>, SceneFileError> {
    let mut file = SceneFile::from_ron(text)?;
    for problem in file.resolve_prefabs(root) {
        tracing::warn!("{problem}");
    }
    Ok(file.entities.iter().map(entity_data_to_staged).collect())
}

/// The asset root, else the working directory: where prefab links resolve.
#[must_use]
pub fn link_root() -> std::path::PathBuf {
    automata_assets::root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}
