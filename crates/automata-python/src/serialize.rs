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
        ..EntityData::default()
    };
    for component in &entity.components {
        match component {
            StagedComponent::Transform(t) => data.transform = Some(TransformData::from(*t)),
            StagedComponent::Camera3D(c) => data.camera_3d = Some(*c),
            StagedComponent::Camera2D(c) => data.camera_2d = Some(*c),
            StagedComponent::DirectionalLight(l) => data.directional_light = Some(*l),
            StagedComponent::Environment(e) => data.environment = Some(*e),
            StagedComponent::AudioSource(a) => data.audio_source = Some(a.clone()),
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
    if let Some(e) = &data.environment {
        components.push(StagedComponent::Environment(*e));
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
    StagedEntity {
        name: data.name.clone(),
        components,
        attributes: data.attributes.clone(),
        script: data.script.as_ref().map(|s| s.path.clone()),
    }
}

/// Serializes a staged scene to RON.
pub fn scene_to_ron(entities: &[StagedEntity]) -> Result<String, SceneFileError> {
    let file = SceneFile {
        version: SCENE_FILE_VERSION,
        entities: entities.iter().map(staged_to_entity_data).collect(),
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
