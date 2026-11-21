//! Graphics quality presets.
//!
//! A preset bundles the quality knobs the renderer supports; games pick one
//! at startup (`Game(graphics="high")` from Python). Finer-grained controls
//! build on this type.

/// A graphics quality preset.
///
/// The preset controls the shadow map: [`GraphicsQuality::Low`] disables
/// shadows entirely; higher presets raise the shadow map resolution.
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
}

impl GraphicsQuality {
    /// Every preset, lowest first.
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Ultra];

    /// A short display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    /// The shadow map side length in texels (`1` when shadows are off; the
    /// texture still exists so bind groups stay valid).
    #[must_use]
    pub fn shadow_map_size(self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Medium => 1024,
            Self::High => 2048,
            Self::Ultra => 4096,
        }
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
    fn default_is_high() {
        assert_eq!(GraphicsQuality::default(), GraphicsQuality::High);
    }
}
