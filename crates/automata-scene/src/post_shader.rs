//! Post shaders: a user WGSL effect drawn over the whole frame.
//!
//! An entity with a [`PostShader`] runs its `shader` file's
//! `fn post(in: PostInput) -> vec3<f32>` on every pixel of the finished
//! frame, under the 2D HUD (the first such entity wins, like
//! [`Environment`](automata_renderer::Environment)). The file loads through
//! the asset cache and is read again when it's saved. A shader that doesn't
//! pass the checks reports why (to the log, and to Oberon's Console) and the
//! frame keeps its standard look.

use automata_renderer::{Renderer, Shader, ShaderKind, SHADER_PARAMS};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::scene::SceneWorld;

/// Draws a user post-effect shader over the frame.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostShader {
    /// The `.wgsl` file (relative to the project, like model paths).
    pub shader: String,
    /// Up to eight numbers the shader reads as `in.p0` (first four) and
    /// `in.p1` (the rest).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<f32>,
}

impl PostShader {
    /// A post shader from its file.
    #[must_use]
    pub fn new(shader: impl Into<String>) -> Self {
        Self {
            shader: shader.into(),
            params: Vec::new(),
        }
    }

    /// With these parameters (past eight are ignored).
    #[must_use]
    pub fn with_params(mut self, params: &[f32]) -> Self {
        self.params = params.iter().copied().take(SHADER_PARAMS).collect();
        self
    }

    /// The parameters as the shader sees them (missing ones are zero).
    #[must_use]
    pub fn values(&self) -> [f32; SHADER_PARAMS] {
        let mut out = [0.0; SHADER_PARAMS];
        for (o, p) in out.iter_mut().zip(&self.params) {
            *o = if p.is_finite() { *p } else { 0.0 };
        }
        out
    }
}

/// The compiled shader of a [`PostShader`] (added by
/// [`crate::materialize_visuals`]).
pub(crate) struct PostShaderGpu {
    key: String,
    asset: Option<Arc<automata_assets::shader::ShaderAsset>>,
    pub(crate) shader: Option<Shader>,
}

/// Compiles the post shaders whose file is new or changed.
pub(crate) fn prepare_post_shaders(world: &mut SceneWorld, renderer: &Renderer) {
    let mut wanted = Vec::new();
    for (entity, post, have) in
        &mut world.query::<(hecs::Entity, &PostShader, Option<&PostShaderGpu>)>()
    {
        let current = have
            .is_some_and(|h| h.key == post.shader && h.asset.as_ref().is_some_and(|a| !a.stale()));
        if !current {
            wanted.push((entity, post.shader.clone()));
        }
    }
    for (entity, key) in wanted {
        let asset = if key.trim().is_empty() {
            None
        } else {
            match automata_assets::shader::load_shader(&key, ShaderKind::Post) {
                Ok(asset) => Some(asset),
                Err(e) => {
                    warn_once(&e);
                    None
                }
            }
        };
        let shader = asset.as_ref().and_then(|a| a.gpu(renderer));
        // A missing file is looked for again next frame (it may be about
        // to be saved), a present one only once it changes.
        let _ = world.insert_one(entity, PostShaderGpu { key, asset, shader });
    }
}

fn warn_once(message: &str) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.iter().any(|s| s == message) {
        tracing::warn!("{message}");
        seen.push(message.to_string());
    }
}

/// Puts the scene's post shader (if it has a working one) in the frame.
pub(crate) fn record_post_shader(world: &SceneWorld, draw: &mut automata_renderer::DrawList) {
    let mut query = world.query::<(&PostShader, &PostShaderGpu)>();
    if let Some((post, gpu)) = (&mut query).into_iter().next() {
        if let Some(shader) = &gpu.shader {
            draw.set_post_shader(shader, post.values());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_shaders_round_trip_and_clamp_their_params() {
        let post = PostShader::new("shaders/crt.wgsl").with_params(&[1.0; 12]);
        assert_eq!(post.params.len(), SHADER_PARAMS);
        let text = ron::to_string(&post).unwrap();
        let back: PostShader = ron::from_str(&text).unwrap();
        assert_eq!(back, post);
        let partial: PostShader = ron::from_str("(shader: \"a.wgsl\")").unwrap();
        assert_eq!(partial.params, Vec::<f32>::new());
        let odd = PostShader {
            shader: String::new(),
            params: vec![0.5, f32::NAN],
        };
        assert_eq!(odd.values()[..3], [0.5, 0.0, 0.0]);
    }
}
