//! Graphics quality presets.
//!
//! A preset bundles the quality knobs the renderer supports; games pick one
//! at startup (`Game(graphics="high")` from Python). Finer-grained controls
//! build on this type.
//!
//! [`GraphicsQuality::Beautiful`] is the top tier: cascaded soft shadows,
//! physically based sun lighting, an HDR pipeline with bloom and FXAA, and
//! refractive water. The other presets keep the cheaper forward path.

/// A graphics quality preset.
///
/// The preset controls the shadow map: [`GraphicsQuality::Low`] disables
/// shadows entirely; higher presets raise the shadow map resolution, and
/// [`GraphicsQuality::Beautiful`] switches on everything else too.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GraphicsQuality {
    /// No shadows — for weak GPUs and stylized flat looks.
    Low,
    /// 1024px shadow map.
    Medium,
    /// 2048px shadow map (the default).
    #[default]
    High,
    /// 4096px shadow map.
    Ultra,
    /// Four 2048px shadow cascades with soft filtering, physically based
    /// lighting, HDR bloom, FXAA and refractive water.
    Beautiful,
}

impl GraphicsQuality {
    /// Every preset, lowest first.
    pub const ALL: [Self; 5] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Ultra,
        Self::Beautiful,
    ];

    /// A short display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
            Self::Beautiful => "Beautiful",
        }
    }

    /// The shadow map side length in texels, per cascade (`1` when shadows
    /// are off; the texture still exists so bind groups stay valid).
    #[must_use]
    pub fn shadow_map_size(self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Medium => 1024,
            Self::High | Self::Beautiful => 2048,
            Self::Ultra => 4096,
        }
    }

    /// How many point lights (glowing meshes included) cast shadows in a
    /// frame: none on Low and Medium, 2 on High, 4 on Ultra and Beautiful.
    #[must_use]
    pub fn point_shadows(self) -> usize {
        match self {
            Self::Low | Self::Medium => 0,
            Self::High => 2,
            Self::Ultra | Self::Beautiful => crate::MAX_POINT_SHADOWS,
        }
    }

    /// Texels per side of each face of a point light's shadow cube.
    #[must_use]
    pub fn point_shadow_size(self) -> u32 {
        match self {
            Self::Low | Self::Medium => 1,
            Self::High => 384,
            Self::Ultra | Self::Beautiful => 512,
        }
    }

    /// How many point lights a frame shades (glowing meshes included);
    /// the ones nearest the camera and brightest win.
    #[must_use]
    pub fn max_point_lights(self) -> usize {
        match self {
            Self::Low => 4,
            Self::Medium => 8,
            Self::High => 16,
            Self::Ultra | Self::Beautiful => crate::MAX_POINT_LIGHTS,
        }
    }

    /// Shadow cascades: `1` is a single map fit around the whole scene;
    /// more split the camera's view so nearby shadows get the most texels.
    #[must_use]
    pub fn shadow_cascades(self) -> u32 {
        match self {
            Self::Beautiful => u32::try_from(crate::shadow::MAX_CASCADES).unwrap_or(1),
            _ => 1,
        }
    }

    /// Whether this preset uses the Beautiful path: soft shadows, physically
    /// based lighting, HDR with bloom and FXAA, and refractive water.
    #[must_use]
    pub fn is_beautiful(self) -> bool {
        matches!(self, Self::Beautiful)
    }

    /// Whether this preset renders shadows at all.
    #[must_use]
    pub fn shadows_enabled(self) -> bool {
        !matches!(self, Self::Low)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_ladders_shadow_resolution() {
        assert!(!GraphicsQuality::Low.shadows_enabled());
        assert!(GraphicsQuality::Medium.shadows_enabled());
        assert!(
            GraphicsQuality::Medium.shadow_map_size() < GraphicsQuality::High.shadow_map_size()
        );
        assert!(GraphicsQuality::High.shadow_map_size() < GraphicsQuality::Ultra.shadow_map_size());
    }

    #[test]
    fn only_beautiful_cascades_and_goes_hdr() {
        for q in GraphicsQuality::ALL {
            assert_eq!(q.is_beautiful(), q == GraphicsQuality::Beautiful);
            let cascades = if q.is_beautiful() { 4 } else { 1 };
            assert_eq!(q.shadow_cascades(), cascades, "{q:?}");
        }
        assert!(GraphicsQuality::Beautiful.shadows_enabled());
    }

    #[test]
    fn default_is_high() {
        assert_eq!(GraphicsQuality::default(), GraphicsQuality::High);
    }
}
