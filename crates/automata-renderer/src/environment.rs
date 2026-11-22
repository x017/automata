//! Scene-wide look settings: the sky and sun, sky/ground ambient, distance
//! fog, exposure, tone mapping, and post effects (colour grading, bloom,
//! vignette, film grain, chromatic aberration).
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
    /// Ambient occlusion strength: how much creases, corners and the
    /// ground under objects darken the ambient light (`0` = off, `1` =
    /// default, up to `2`). Beautiful preset only.
    pub ambient_occlusion: f32,
    /// Height fog: the top of the fog layer (world Y). Below it the fog
    /// has its full `fog_density`; above it, it thins by `fog_falloff`.
    pub fog_height: f32,
    /// How fast the fog thins above `fog_height`, per metre (`0` = the
    /// same everywhere, plain distance fog; `0.1` = to a third in 11 m).
    pub fog_falloff: f32,
    /// How much the sun lights the fog: it glows toward the sun (`0` =
    /// none, `1` = strong).
    pub fog_sun: f32,
    /// Light shafts from the sun through gaps in the scenery (`0` = off,
    /// `1` = default, up to `2`). Beautiful preset only.
    pub god_rays: f32,
    /// Depth of field: blur in front of and behind the focus (`0` = off,
    /// `1` = default, up to `4`). Beautiful preset only.
    pub depth_of_field: f32,
    /// The distance in focus, in metres (`0` = auto: the camera's target).
    pub focus_distance: f32,
    /// How deep the sharp zone around the focus distance is, in metres.
    pub focus_range: f32,
    /// Colour grading and screen effects, applied after tone mapping on
    /// Medium graphics and up.
    pub post: PostEffects,
}

/// Post effects: bloom, colour grading and lens/film effects.
///
/// The default changes nothing (bloom 1 is the Beautiful preset's usual
/// glow). Medium graphics and up grade the image and draw the vignette,
/// grain and chromatic aberration; High adds a cheap bloom of the
/// brightest pixels; Beautiful's bloom is the full HDR one. Low skips
/// them all.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PostEffects {
    /// Glow around bright things (`0` = none, `1` = default, up to `4`).
    /// High and Beautiful.
    pub bloom: f32,
    /// Contrast around mid grey (`1` = unchanged, `0.5`..`2`).
    pub contrast: f32,
    /// Colour saturation (`0` = black and white, `1` = unchanged, up to `2`).
    pub saturation: f32,
    /// White balance: `-1` (cool, blue) to `1` (warm, orange).
    pub temperature: f32,
    /// White balance: `-1` (green) to `1` (magenta).
    pub tint: f32,
    /// Added to the shadows (linear RGB offset, `0` = none). Raising it
    /// washes the blacks out; colour it to tint the shadows.
    pub lift: [f32; 3],
    /// Midtone power per channel (`1` = unchanged, above 1 brightens).
    pub gamma: [f32; 3],
    /// Highlight multiplier per channel (`1` = unchanged).
    pub gain: [f32; 3],
    /// Darkens the corners (`0` = none, `1` = strong).
    pub vignette: f32,
    /// Animated film grain (`0` = none, `1` = heavy).
    pub grain: f32,
    /// Colour fringes toward the edges of the frame, like a cheap lens
    /// (`0` = none, `1` = strong).
    pub chromatic_aberration: f32,
}

impl Default for PostEffects {
    fn default() -> Self {
        Self {
            bloom: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
            vignette: 0.0,
            grain: 0.0,
            chromatic_aberration: 0.0,
        }
    }
}

impl PostEffects {
    /// Every named look, in UI order: name and settings.
    #[allow(clippy::type_complexity)]
    pub const PRESETS: [(&'static str, fn() -> Self); 5] = [
        ("Neutral", Self::default),
        ("Bleached", Self::bleached),
        ("Vivid", Self::vivid),
        ("Film", Self::film),
        ("Noir", Self::noir),
    ];

    /// A named look (case-insensitive): `"neutral"`, `"bleached"`,
    /// `"vivid"`, `"film"` or `"noir"`.
    #[must_use]
    pub fn preset(name: &str) -> Option<Self> {
        Self::PRESETS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, make)| make())
    }

    /// Washed out and overexposed, with strong soft bloom: pale, nearly
    /// colourless, sun-bleached light.
    #[must_use]
    pub fn bleached() -> Self {
        Self {
            bloom: 2.2,
            contrast: 0.9,
            saturation: 0.5,
            temperature: -0.04,
            tint: 0.0,
            lift: [0.05, 0.05, 0.055],
            gamma: [1.06, 1.06, 1.05],
            gain: [1.18, 1.18, 1.16],
            vignette: 0.25,
            grain: 0.12,
            chromatic_aberration: 0.25,
        }
    }

    /// Warm and saturated, sunny: rich greens and blues, golden light.
    #[must_use]
    pub fn vivid() -> Self {
        Self {
            bloom: 1.4,
            contrast: 1.08,
            saturation: 1.35,
            temperature: 0.14,
            tint: -0.02,
            lift: [0.01, 0.008, 0.0],
            gamma: [1.03, 1.02, 1.0],
            gain: [1.04, 1.02, 0.98],
            vignette: 0.15,
            grain: 0.0,
            chromatic_aberration: 0.0,
        }
    }

    /// Filmic: a touch of contrast and warmth, lifted shadows, grain and a
    /// vignette.
    #[must_use]
    pub fn film() -> Self {
        Self {
            bloom: 1.2,
            contrast: 1.12,
            saturation: 0.88,
            temperature: 0.1,
            tint: 0.02,
            lift: [0.02, 0.02, 0.035],
            gamma: [1.0; 3],
            gain: [1.02, 1.0, 0.96],
            vignette: 0.4,
            grain: 0.3,
            chromatic_aberration: 0.15,
        }
    }

    /// Black and white, hard contrast, heavy vignette and grain.
    #[must_use]
    pub fn noir() -> Self {
        Self {
            bloom: 1.0,
            contrast: 1.35,
            saturation: 0.0,
            temperature: 0.0,
            tint: 0.0,
            lift: [0.0; 3],
            gamma: [0.95; 3],
            gain: [1.05; 3],
            vignette: 0.6,
            grain: 0.35,
            chromatic_aberration: 0.0,
        }
    }

    /// Whether the grading and screen effects change nothing (bloom aside).
    #[must_use]
    pub fn is_neutral_grade(&self) -> bool {
        let n = Self::default();
        Self {
            bloom: n.bloom,
            ..*self
        } == n
    }
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
            ambient_occlusion: 1.0,
            fog_height: 0.0,
            fog_falloff: 0.0,
            fog_sun: 0.0,
            god_rays: 0.0,
            depth_of_field: 0.0,
            focus_distance: 0.0,
            focus_range: 1.0,
            post: PostEffects::default(),
        }
    }
}

impl Environment {
    /// A daylight preset: blue sky ambient, warm ground bounce, light haze
    /// lying low over the ground, soft god rays, ACES tone mapping.
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
            fog_height: 2.0,
            fog_falloff: 0.15,
            fog_sun: 0.5,
            god_rays: 0.5,
            sky: true,
            zenith_color: [0.16, 0.34, 0.78],
            clouds: 0.35,
            ..Self::default()
        }
    }

    /// A dusk preset: orange-violet ambient, dense warm fog pooling in the
    /// low ground and glowing toward the sun, strong god rays, `AgX`.
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
            fog_height: 1.0,
            fog_falloff: 0.3,
            fog_sun: 1.5,
            god_rays: 1.0,
            sky: true,
            zenith_color: [0.1, 0.09, 0.24],
            sun_size: 2.6,
            sun_glow: 1.6,
            clouds: 0.25,
            ..Self::default()
        }
    }

    /// A night preset: cold dark ambient, black fog thickest low down, ACES.
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
            fog_height: 1.0,
            fog_falloff: 0.2,
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
    fn post_effects_default_to_nothing_and_load_partially() {
        let post = PostEffects::default();
        assert!(post.is_neutral_grade());
        assert!(PostEffects { bloom: 0.0, ..post }.is_neutral_grade());
        assert!(!PostEffects {
            vignette: 0.2,
            ..post
        }
        .is_neutral_grade());
        // Old scenes (no post) and partial ones load with the rest neutral.
        let old: Environment = ron::from_str("(exposure: 1.2)").unwrap();
        assert_eq!(old.post, post);
        let some: Environment = ron::from_str("(post: (saturation: 0.5))").unwrap();
        assert!((some.post.saturation - 0.5).abs() < 1e-6);
        assert_eq!(some.post.gain, [1.0; 3]);
    }

    #[test]
    fn post_presets_are_named_and_distinct() {
        assert_eq!(PostEffects::preset("NEUTRAL"), Some(PostEffects::default()));
        assert!(PostEffects::preset("sepia").is_none());
        let bleached = PostEffects::preset("bleached").unwrap();
        let vivid = PostEffects::preset("vivid").unwrap();
        // Bleached: desaturated and brightened; vivid: saturated and warm.
        assert!(bleached.saturation < 0.7 && bleached.gain[0] > 1.1);
        assert!(vivid.saturation > 1.2 && vivid.temperature > 0.1);
        for (name, make) in PostEffects::PRESETS.iter().skip(1) {
            assert!(!make().is_neutral_grade(), "{name}");
        }
    }

    #[test]
    fn modes_have_distinct_indices() {
        let mut seen: Vec<f32> = Tonemapping::ALL.iter().map(|t| t.index()).collect();
        seen.dedup();
        assert_eq!(seen.len(), Tonemapping::ALL.len());
    }
}
