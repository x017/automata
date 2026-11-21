//! Scene-wide look settings: the sky and sun, sky/ground ambient, distance
//! fog, exposure, and tone mapping.
//!
//! An [`Environment`] is set per frame with
//! [`DrawList::set_environment`](crate::DrawList::set_environment). The
//! default reproduces the engine's original look exactly (white ambient,
//! no fog, exposure 1, no tone mapping), so frames without one are
//! unchanged.

/// How lit colors are mapped to the display range.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tonemapping {
    /// Clamp only: colors above 1.0 clip (the original look).
    #[default]
    None,
    /// Reinhard `c / (1 + c)`: soft, low contrast.
    Reinhard,
    /// ACES filmic fit (Narkowicz): punchy contrast, warm highlights.
    Aces,
    /// AgX-style curve: gentle, desaturates bright lights naturally.
    Agx,
}

impl Tonemapping {
    /// Every mode, in UI order.
    pub const ALL: [Self; 4] = [Self::None, Self::Reinhard, Self::Aces, Self::Agx];

    /// A short display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Reinhard => "Reinhard",
            Self::Aces => "ACES",
            Self::Agx => "AgX",
        }
    }

    /// The shader's mode index.
    #[must_use]
    pub(crate) fn index(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Reinhard => 1.0,
            Self::Aces => 2.0,
            Self::Agx => 3.0,
        }
    }
}

/// Scene-wide look settings.
///
/// The fields are plain data so scene files can store them
/// (`environment: (exposure: 1.2, tonemapping: Aces, ..)`); missing fields
/// fall back to [`Environment::default`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Environment {
    /// Ambient light from above (linear RGB, multiplied by the sun's
    /// ambient fraction). Upward-facing surfaces get this color.
    pub sky_color: [f32; 3],
    /// Ambient light from below. Downward-facing surfaces get this color;
    /// in between, the two blend by the surface normal.
    pub ground_color: [f32; 3],
    /// The color behind all geometry (the clear color), linear RGB.
    pub background: [f32; 3],
    /// Brightness multiplier applied before tone mapping (`1.0` = none).
    pub exposure: f32,
    /// The tone mapping curve.
    pub tonemapping: Tonemapping,
    /// Whether distance fog is applied.
    pub fog: bool,
    /// Fog color (linear RGB). Usually matches `background`.
    pub fog_color: [f32; 3],
    /// Exponential fog density per world unit (`0.02` ≈ thick at 100 m).
    pub fog_density: f32,
    /// Distance from the camera where fog starts.
    pub fog_start: f32,
    /// Draws a sky dome with a sun behind the scene instead of the flat
    /// `background`, which becomes the horizon color. Reflections and fog
    /// then take their colors from the same sky, so distant things fade
    /// into the horizon.
    pub sky: bool,
    /// The sky's color straight up (linear RGB).
    pub zenith_color: [f32; 3],
    /// The sun disk's apparent diameter in degrees (the real sun is about
    /// 0.53; games usually draw it larger).
    pub sun_size: f32,
    /// Brightness of the halo around the sun (`0` = none, `1` = default).
    pub sun_glow: f32,
    /// Cloud cover, `0` (clear) to `1` (overcast).
    pub clouds: f32,
    /// Lens flare and glare when looking toward the sun (`0` = off). Drawn
    /// on the Beautiful preset only, and hidden when something blocks the
    /// sun.
    pub lens_flare: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_color: [1.0, 1.0, 1.0],
            ground_color: [1.0, 1.0, 1.0],
            background: [0.012, 0.012, 0.012],
            exposure: 1.0,
            tonemapping: Tonemapping::None,
            fog: false,
            fog_color: [0.5, 0.55, 0.6],
            fog_density: 0.02,
            fog_start: 0.0,
            sky: false,
            zenith_color: [0.16, 0.34, 0.78],
            sun_size: 2.0,
            sun_glow: 1.0,
            clouds: 0.0,
            lens_flare: 1.0,
        }
    }
}

impl Environment {
    /// A daylight preset: blue sky ambient, warm ground bounce, light haze,
    /// ACES tone mapping.
    #[must_use]
    pub fn daylight() -> Self {
        Self {
            sky_color: [0.75, 0.85, 1.0],
            ground_color: [0.45, 0.4, 0.35],
            background: [0.55, 0.68, 0.85],
            exposure: 1.1,
            tonemapping: Tonemapping::Aces,
            fog: true,
            fog_color: [0.55, 0.68, 0.85],
            fog_density: 0.012,
            fog_start: 10.0,
            sky: true,
            zenith_color: [0.16, 0.34, 0.78],
            clouds: 0.35,
            ..Self::default()
        }
    }

    /// A dusk preset: orange-violet ambient, dense warm fog, `AgX`.
    #[must_use]
    pub fn dusk() -> Self {
        Self {
            sky_color: [0.55, 0.45, 0.7],
            ground_color: [0.35, 0.2, 0.15],
            background: [0.32, 0.18, 0.2],
            exposure: 1.3,
            tonemapping: Tonemapping::Agx,
            fog: true,
            fog_color: [0.32, 0.18, 0.2],
            fog_density: 0.03,
            fog_start: 4.0,
            sky: true,
            zenith_color: [0.1, 0.09, 0.24],
            sun_size: 2.6,
            sun_glow: 1.6,
            clouds: 0.25,
            ..Self::default()
        }
    }

    /// A night preset: cold dark ambient, black fog, ACES.
    #[must_use]
    pub fn night() -> Self {
        Self {
            sky_color: [0.25, 0.3, 0.5],
            ground_color: [0.05, 0.05, 0.08],
            background: [0.008, 0.01, 0.02],
            exposure: 1.0,
            tonemapping: Tonemapping::Aces,
            fog: true,
            fog_color: [0.008, 0.01, 0.02],
            fog_density: 0.05,
            fog_start: 2.0,
            sky: true,
            zenith_color: [0.002, 0.003, 0.01],
            sun_size: 1.6,
            sun_glow: 0.3,
            lens_flare: 0.0,
            ..Self::default()
        }
    }

    /// The background as a clear color.
    #[must_use]
    pub fn clear_color(&self) -> wgpu::Color {
        let c = |i: usize| f64::from(self.background[i]);
        wgpu::Color {
            r: c(0),
            g: c(1),
            b: c(2),
            a: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_legacy_look() {
        let env = Environment::default();
        assert_eq!(env.sky_color, [1.0; 3]);
        assert_eq!(env.ground_color, [1.0; 3]);
        assert!((env.exposure - 1.0).abs() < f32::EPSILON);
        assert_eq!(env.tonemapping, Tonemapping::None);
        assert!(!env.fog);
        assert!(!env.sky, "no sky dome unless asked for");
    }

    #[test]
    fn presets_draw_a_sky_and_old_scenes_keep_theirs() {
        assert!(Environment::daylight().sky && Environment::daylight().clouds > 0.0);
        assert!(Environment::dusk().sky);
        // A scene saved before the sky existed loads without one.
        let old: Environment = ron::from_str("(exposure: 1.2, fog: true)").unwrap();
        assert!(!old.sky);
        assert!((old.exposure - 1.2).abs() < 1e-6);
    }

    #[test]
    fn modes_have_distinct_indices() {
        let mut seen: Vec<f32> = Tonemapping::ALL.iter().map(|t| t.index()).collect();
        seen.dedup();
        assert_eq!(seen.len(), Tonemapping::ALL.len());
    }
}
