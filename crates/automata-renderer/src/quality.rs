//! Graphics quality presets.
//!
//! A preset bundles the quality knobs the renderer supports; games pick one
//! at startup (`Game(graphics="high")` from Python). Finer-grained controls
//! build on this type.
//!
//! [`GraphicsQuality::Beautiful`] is the top tier: soft shadows, physically
//! based sun lighting, an HDR pipeline with bloom and FXAA, and refractive
//! water. The other presets keep the cheaper forward path; High and Ultra
//! still split their sun shadows into cascades so nearby shadows are crisp.

/// A graphics quality preset.
///
/// The preset controls the shadow maps: [`GraphicsQuality::Low`] disables
/// shadows entirely; higher presets add resolution and cascades, and
/// [`GraphicsQuality::Beautiful`] switches on everything else too.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GraphicsQuality {
    /// No shadows — for weak GPUs and stylized flat looks.
    Low,
    /// One 1024px shadow map.
    Medium,
    /// Two 2048px shadow cascades (the default).
    #[default]
    High,
    /// Three 2048px shadow cascades.
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
            Self::High | Self::Ultra | Self::Beautiful => 2048,
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

    /// Sun shadow cascades: each splits off a slice of the camera's view
    /// (up to the sun's shadow distance) with its own map, so nearby
    /// shadows get the most texels. With one, the map covers the whole
    /// scene or the view, whichever is smaller.
    #[must_use]
    pub fn shadow_cascades(self) -> u32 {
        match self {
            Self::Low | Self::Medium => 1,
            Self::High => 2,
            Self::Ultra => 3,
            Self::Beautiful => u32::try_from(crate::shadow::MAX_CASCADES).unwrap_or(1),
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
        let cascades: Vec<u32> = GraphicsQuality::ALL
            .iter()
            .map(|q| q.shadow_cascades())
            .collect();
        assert_eq!(cascades, [1, 1, 2, 3, 4]);
    }

    #[test]
    fn only_beautiful_goes_hdr() {
        for q in GraphicsQuality::ALL {
            assert_eq!(q.is_beautiful(), q == GraphicsQuality::Beautiful);
        }
        assert!(GraphicsQuality::Beautiful.shadows_enabled());
    }

    #[test]
    fn default_is_high() {
        assert_eq!(GraphicsQuality::default(), GraphicsQuality::High);
    }
}
