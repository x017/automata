//! Art-direction styles: named sets of the [`Environment`] knobs that make
//! up a look (tone mapping, exposure, atmosphere, shading, outlines,
//! pixelation and post effects), leaving the scene's own colours, fog
//! colour and density, sky and clouds alone.
//!
//! Styles mix: apply one, then change any knob. An Environment "is" a
//! style while applying that style again would change nothing, so the
//! editor's Style menu shows "Custom" as soon as a style knob moves.

use crate::{Environment, PostEffects, Tonemapping};

/// The knobs a style owns.
#[derive(Debug, Clone, Copy)]
struct Look {
    tonemapping: Tonemapping,
    exposure: f32,
    ambient_occlusion: f32,
    lens_flare: f32,
    fog_height: f32,
    fog_falloff: f32,
    fog_sun: f32,
    god_rays: f32,
    depth_of_field: f32,
    focus_distance: f32,
    focus_range: f32,
    toon_bands: u32,
    toon_softness: f32,
    shadow_tint: [f32; 3],
    rim_light: f32,
    painted_sky: bool,
    outline: f32,
    outline_color: [f32; 3],
    pixelate: f32,
    palette: u32,
    post: PostEffects,
}

impl Look {
    fn apply(self, env: Environment) -> Environment {
        Environment {
            tonemapping: self.tonemapping,
            exposure: self.exposure,
            ambient_occlusion: self.ambient_occlusion,
            lens_flare: self.lens_flare,
            fog_height: self.fog_height,
            fog_falloff: self.fog_falloff,
            fog_sun: self.fog_sun,
            god_rays: self.god_rays,
            depth_of_field: self.depth_of_field,
            focus_distance: self.focus_distance,
            focus_range: self.focus_range,
            toon_bands: self.toon_bands,
            toon_softness: self.toon_softness,
            shadow_tint: self.shadow_tint,
            rim_light: self.rim_light,
            painted_sky: self.painted_sky,
            outline: self.outline,
            outline_color: self.outline_color,
            pixelate: self.pixelate,
            palette: self.palette,
            post: self.post,
            ..env
        }
    }
}

/// Physically based light, filmic tone mapping, a little haze and soft
/// sun shafts, no grade.
fn realistic() -> Look {
    Look {
        tonemapping: Tonemapping::Aces,
        exposure: 1.0,
        ambient_occlusion: 1.0,
        lens_flare: 1.0,
        fog_height: 2.0,
        fog_falloff: 0.1,
        fog_sun: 0.5,
        god_rays: 0.5,
        depth_of_field: 0.0,
        focus_distance: 0.0,
        focus_range: 1.0,
        toon_bands: 0,
        toon_softness: 0.15,
        shadow_tint: [1.0; 3],
        rim_light: 0.0,
        painted_sky: false,
        outline: 0.0,
        outline_color: [0.05, 0.04, 0.06],
        pixelate: 0.0,
        palette: 0,
        post: PostEffects::default(),
    }
}

/// Realistic light pushed toward a game look: softer filmic curve, cool
/// shade, a faint rim, a little more colour and glow.
fn semi_realistic() -> Look {
    Look {
        tonemapping: Tonemapping::Agx,
        exposure: 1.2,
        lens_flare: 0.7,
        fog_falloff: 0.15,
        fog_sun: 0.7,
        god_rays: 0.6,
        depth_of_field: 0.3,
        focus_range: 10.0,
        shadow_tint: [0.88, 0.92, 1.0],
        rim_light: 0.35,
        post: PostEffects {
            bloom: 1.3,
            contrast: 1.06,
            saturation: 1.12,
            temperature: 0.04,
            vignette: 0.15,
            ..PostEffects::default()
        },
        ..realistic()
    }
}

/// Cel shading with a soft terminator, blue-tinted shade, a bright rim,
/// a painted sky and saturated, glowing colour.
fn storybook() -> Look {
    Look {
        exposure: 1.1,
        ambient_occlusion: 0.6,
        lens_flare: 0.4,
        fog_falloff: 0.15,
        fog_sun: 0.8,
        toon_bands: 1,
        toon_softness: 0.3,
        shadow_tint: [0.72, 0.8, 1.0],
        rim_light: 0.9,
        painted_sky: true,
        post: PostEffects {
            bloom: 1.6,
            saturation: 1.2,
            temperature: 0.05,
            lift: [0.02, 0.03, 0.05],
            gain: [1.04, 1.02, 1.0],
            ..PostEffects::default()
        },
        ..realistic()
    }
}

/// Low-resolution pixel art: big pixels, hard two-band toon shading, thin
/// dark outlines, a painted sky, no lens effects.
fn pixel_hike() -> Look {
    Look {
        exposure: 1.1,
        ambient_occlusion: 0.0,
        lens_flare: 0.0,
        fog_sun: 0.4,
        god_rays: 0.0,
        toon_bands: 2,
        toon_softness: 0.05,
        shadow_tint: [0.75, 0.78, 1.0],
        rim_light: 0.4,
        painted_sky: true,
        outline: 1.0,
        outline_color: [0.08, 0.06, 0.1],
        pixelate: 3.0,
        post: PostEffects {
            bloom: 0.8,
            contrast: 1.05,
            saturation: 1.15,
            temperature: 0.03,
            ..PostEffects::default()
        },
        ..realistic()
    }
}

/// Washed out and overexposed: pale colour, strong soft bloom, thick haze
/// low down, heavy sun shafts, a soft focus, grain, fringes and vignette.
fn hazy() -> Look {
    Look {
        exposure: 1.25,
        lens_flare: 0.6,
        fog_height: 2.5,
        fog_falloff: 0.2,
        fog_sun: 1.2,
        god_rays: 1.0,
        depth_of_field: 0.5,
        focus_range: 8.0,
        rim_light: 0.15,
        post: PostEffects {
            bloom: 2.0,
            contrast: 0.92,
            saturation: 0.62,
            temperature: 0.03,
            lift: [0.04, 0.04, 0.045],
            gain: [1.12, 1.1, 1.06],
            vignette: 0.35,
            grain: 0.22,
            chromatic_aberration: 0.22,
            ..PostEffects::default()
        },
        ..realistic()
    }
}

impl Environment {
    /// Every style in UI order: name, then the extra names
    /// [`Environment::with_style`] accepts.
    pub const STYLES: [(&'static str, &'static [&'static str]); 5] = [
        ("Realistic", &[]),
        ("Semi-realistic", &["semi"]),
        ("BotW", &["storybook", "toon"]),
        ("A Short Hike", &["short hike", "pixel"]),
        ("NieR", &["hazy"]),
    ];

    /// This Environment with style `name` applied (case, spaces, dashes
    /// and underscores don't matter: `"nier"`, `"short_hike"`,
    /// `"Semi-realistic"`), or `None` for an unknown name. Only the style's
    /// own knobs change; the scene's colours, fog and sky stay.
    #[must_use]
    pub fn with_style(self, name: &str) -> Option<Self> {
        let key = style_key(name);
        let index = Self::STYLES.iter().position(|(n, aliases)| {
            style_key(n) == key || aliases.iter().any(|a| style_key(a) == key)
        })?;
        let look = match index {
            0 => realistic(),
            1 => semi_realistic(),
            2 => storybook(),
            3 => pixel_hike(),
            _ => hazy(),
        };
        Some(look.apply(self))
    }

    /// The name of the style this Environment is exactly (applying it
    /// would change nothing), or `"Custom"`.
    #[must_use]
    pub fn style_name(&self) -> &'static str {
        Self::STYLES
            .iter()
            .find(|(name, _)| self.with_style(name) == Some(*self))
            .map_or("Custom", |(name, _)| name)
    }
}

/// `name` folded for matching: lowercase letters and digits only.
fn style_key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_apply_by_any_spelling_and_are_recognised() {
        let base = Environment::daylight();
        for (name, _) in Environment::STYLES {
            let styled = base.with_style(name).unwrap();
            assert_eq!(styled.style_name(), name);
            // The scene's own look stays.
            assert_eq!(styled.background, base.background);
            assert_eq!(styled.fog_density, base.fog_density);
            assert_eq!(styled.clouds, base.clouds);
        }
        let short = base.with_style("short_hike").unwrap();
        assert_eq!(short, base.with_style("A Short Hike").unwrap());
        assert_eq!(short, base.with_style("pixel").unwrap());
        assert_eq!(base.with_style("NIER"), base.with_style("hazy"));
        assert!(base.with_style("vaporwave").is_none());
        // Any style knob moved: custom. Applying another style replaces
        // every knob the first set.
        let tweaked = Environment {
            outline: 2.0,
            ..short
        };
        assert_eq!(tweaked.style_name(), "Custom");
        let back = short.with_style("realistic").unwrap();
        assert_eq!(back.style_name(), "Realistic");
        assert!(back.pixelate == 0.0 && back.toon_bands == 0 && !back.painted_sky);
    }

    #[test]
    fn the_styles_look_different() {
        let base = Environment::daylight();
        let s = |n: &str| base.with_style(n).unwrap();
        assert!(s("botw").toon_bands > 0 && s("botw").painted_sky && s("botw").rim_light > 0.5);
        assert!(s("short hike").pixelate > 1.0 && s("short hike").outline > 0.0);
        assert!(s("nier").post.saturation < 0.8 && s("nier").god_rays > 0.5);
        assert!(s("nier").depth_of_field > 0.0 && s("nier").post.grain > 0.0);
        assert!(s("realistic").toon_bands == 0 && s("realistic").post.is_neutral_grade());
        assert!(!s("semi-realistic").post.is_neutral_grade());
    }
}
