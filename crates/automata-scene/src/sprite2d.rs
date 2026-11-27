//! 2D: sprites (with sprite-sheet animation), tilemaps drawn as sprites,
//! and a 2D camera that follows an entity.
//!
//! Sprites and tilemaps are plain data ([`Sprite2D`], [`crate::Tilemap`]),
//! so scene files, scripts and the editor stage them before a renderer
//! exists; [`crate::materialize_visuals`] loads their images and
//! [`crate::record_draw_list`] draws them through the scene's
//! [`Camera2D`], sorted by `layer` and then by Z (higher in front).

use automata_renderer::{Camera2D, DrawList, Renderer, Sprite, Texture};
use glam::{EulerRot, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::{Name, SceneWorld};
use crate::tilemap::{frame_uv, Tilemap};
use crate::transform::{GlobalTransform, Transform};

/// A named run of frames in a sprite sheet: `first..=last`, played at
/// `fps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpriteClip {
    /// What scripts play it by (`entity.sprite.play("run")`).
    pub name: String,
    /// The first frame.
    pub first: u32,
    /// The last frame (inclusive).
    pub last: u32,
    /// Frames per second.
    pub fps: f32,
    /// Starts over at the end; otherwise it holds the last frame.
    pub looping: bool,
}

impl Default for SpriteClip {
    fn default() -> Self {
        Self {
            name: "clip".into(),
            first: 0,
            last: 0,
            fps: 10.0,
            looping: true,
        }
    }
}

impl SpriteClip {
    /// A looping clip.
    #[must_use]
    pub fn new(name: &str, first: u32, last: u32, fps: f32) -> Self {
        Self {
            name: name.into(),
            first,
            last,
            fps,
            looping: true,
        }
    }

    /// Frames in the clip.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.last.saturating_sub(self.first) + 1
    }

    /// Never empty (a clip has at least its first frame).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Seconds one pass takes.
    #[must_use]
    pub fn duration(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)] // small counts
        let frames = self.len() as f32;
        frames / self.fps.max(1e-3)
    }

    /// The frame `time` seconds in, and whether a one-shot clip has ended.
    #[must_use]
    pub fn frame_at(&self, time: f32) -> (u32, bool) {
        let len = self.len();
        // Non-negative and finite: fits u64.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = (time.max(0.0) * self.fps.max(0.0)).floor() as u64;
        if self.looping {
            let i = u32::try_from(step % u64::from(len)).unwrap_or(0);
            (self.first + i, false)
        } else {
            let i = u32::try_from(step.min(u64::from(len - 1))).unwrap_or(0);
            (self.first + i, step >= u64::from(len))
        }
    }
}

/// A 2D image on the entity: centred on its position, turned by its Z
/// rotation, sized `size` times its scale. Optionally one frame of a sprite
/// sheet (`columns × rows` equal frames), animated by `clips`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // flips and play state, as saved
pub struct Sprite2D {
    /// The image (relative to the scene, like model paths). Empty draws a
    /// plain `color` rectangle.
    pub image: String,
    /// Width and height in world units (before the entity's scale).
    pub size: [f32; 2],
    /// Tint multiplied with the image.
    pub color: [f32; 4],
    /// Mirrored left to right.
    pub flip_x: bool,
    /// Mirrored top to bottom.
    pub flip_y: bool,
    /// Hard pixel edges (pixel art) instead of smooth filtering.
    pub pixelated: bool,
    /// Drawing order: higher layers draw over lower ones (then higher Z
    /// over lower).
    pub layer: i32,
    /// Sheet frames across.
    pub columns: u32,
    /// Sheet frames down.
    pub rows: u32,
    /// The frame shown, counting left to right, top to bottom (clips set
    /// it while they play).
    pub frame: u32,
    /// Named frame runs scripts play (`entity.sprite.play("run")`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<SpriteClip>,
    /// The clip playing from the start ("" for none).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub play: String,
    /// While running: the clip playing now ("" for none).
    #[serde(skip)]
    pub playing: String,
    /// Seconds into the playing clip.
    #[serde(skip)]
    pub clip_time: f32,
    /// A one-shot clip reached its end.
    #[serde(skip)]
    pub finished: bool,
    /// Whether `playing` was set up from `play` yet.
    #[serde(skip)]
    pub started: bool,
}

impl Default for Sprite2D {
    fn default() -> Self {
        Self {
            image: String::new(),
            size: [1.0, 1.0],
            color: [1.0; 4],
            flip_x: false,
            flip_y: false,
            pixelated: true,
            layer: 0,
            columns: 1,
            rows: 1,
            frame: 0,
            clips: Vec::new(),
            play: String::new(),
            playing: String::new(),
            clip_time: 0.0,
            finished: false,
            started: false,
        }
    }
}

impl Sprite2D {
    /// A 1×1 sprite showing `image`.
    #[must_use]
    pub fn new(image: &str) -> Self {
        Self {
            image: image.into(),
            ..Self::default()
        }
    }

    /// The clip called `name`.
    #[must_use]
    pub fn clip(&self, name: &str) -> Option<&SpriteClip> {
        self.clips.iter().find(|c| c.name == name)
    }

    /// Starts clip `name` (`restart`: from its first frame even when it's
    /// already playing). Returns false when there's no such clip.
    pub fn play_clip(&mut self, name: &str, restart: bool) -> bool {
        self.started = true;
        let Some(clip) = self.clip(name) else {
            return false;
        };
        if restart || self.playing != name {
            self.frame = clip.first;
            self.playing = name.into();
            self.clip_time = 0.0;
            self.finished = false;
        }
        true
    }

    /// Stops the clip, holding the frame shown.
    pub fn stop(&mut self) {
        self.started = true;
        self.playing.clear();
    }

    /// Advances the playing clip by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        if !self.started {
            self.started = true;
            let play = self.play.clone();
            if !play.is_empty() {
                self.play_clip(&play, true);
            }
        }
        if self.playing.is_empty() {
            return;
        }
        let Some(clip) = self.clip(&self.playing).cloned() else {
            return;
        };
        self.clip_time += dt.max(0.0);
        if clip.looping {
            self.clip_time %= clip.duration().max(1e-3);
        }
        (self.frame, self.finished) = clip.frame_at(self.clip_time);
    }

    /// The image's texture window for the current frame, flips applied.
    #[must_use]
    pub fn uv(&self) -> [f32; 4] {
        let [u0, v0, u1, v1] = frame_uv(self.frame, self.columns, self.rows);
        let (u0, u1) = if self.flip_x { (u1, u0) } else { (u0, u1) };
        let (v0, v1) = if self.flip_y { (v1, v0) } else { (v0, v1) };
        [u0, v0, u1, v1]
    }
}

/// A script's changes to a sprite, applied after its frame (see
/// [`SpriteEdit::apply`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpriteEdit {
    /// Play this clip (and whether to restart it if it's playing).
    pub play: Option<(String, bool)>,
    /// Stop the clip, holding its frame.
    pub stop: bool,
    /// Mirror left to right.
    pub flip_x: Option<bool>,
    /// Mirror top to bottom.
    pub flip_y: Option<bool>,
    /// Show this frame (stops the clip).
    pub frame: Option<u32>,
    /// Retint.
    pub color: Option<[f32; 4]>,
    /// Move to this drawing layer.
    pub layer: Option<i32>,
    /// Swap the image.
    pub image: Option<String>,
    /// Resize (world units).
    pub size: Option<[f32; 2]>,
}

impl SpriteEdit {
    /// Applies the changes, in a fixed order: the image and look first,
    /// then stopping, a frame, and playing.
    pub fn apply(&self, sprite: &mut Sprite2D) {
        if let Some(image) = &self.image {
            sprite.image.clone_from(image);
        }
        if let Some(size) = self.size {
            sprite.size = size;
        }
        if let Some(color) = self.color {
            sprite.color = color;
        }
        if let Some(layer) = self.layer {
            sprite.layer = layer;
        }
        if let Some(f) = self.flip_x {
            sprite.flip_x = f;
        }
        if let Some(f) = self.flip_y {
            sprite.flip_y = f;
        }
        if self.stop {
            sprite.stop();
        }
        if let Some(frame) = self.frame {
            sprite.stop();
            sprite.frame = frame;
        }
        if let Some((clip, restart)) = &self.play {
            sprite.play_clip(clip, *restart);
        }
    }
}

/// Follows an entity with the 2D camera it sits on (with the camera's
/// [`Camera2D`]): the camera's position eases toward the target's (plus
/// `offset`) while the target leaves the `dead_zone`, kept inside
/// `bounds` when they're set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraFollow2D {
    /// The followed entity's name.
    pub target: String,
    /// Where the camera looks relative to the target.
    pub offset: [f32; 2],
    /// Lag in seconds (roughly how long it takes to catch up); 0 sticks
    /// to the target.
    pub smoothing: f32,
    /// Half the size of a box around the view's centre the target moves
    /// in freely.
    pub dead_zone: [f32; 2],
    /// `[min x, min y, max x, max y]` the view stays inside (assuming a
    /// 16:9 window); all zero for no limits.
    pub bounds: [f32; 4],
    /// Snapped onto the target yet (the first frame doesn't ease).
    #[serde(skip)]
    pub settled: bool,
}

impl Default for CameraFollow2D {
    fn default() -> Self {
        Self {
            target: "Player".into(),
            offset: [0.0, 1.0],
            smoothing: 0.15,
            dead_zone: [0.5, 0.5],
            bounds: [0.0; 4],
            settled: false,
        }
    }
}

impl CameraFollow2D {
    /// Follows the entity called `target`.
    #[must_use]
    pub fn new(target: &str) -> Self {
        Self {
            target: target.into(),
            ..Self::default()
        }
    }

    /// Whether `bounds` limit the view.
    #[must_use]
    pub fn bounded(&self) -> bool {
        self.bounds[2] > self.bounds[0] || self.bounds[3] > self.bounds[1]
    }

    /// The next view centre, from `current`, toward `target` over `dt`
    /// seconds, for a view `half_height` tall.
    #[must_use]
    pub fn step(&self, current: Vec2, target: Vec2, half_height: f32, dt: f32) -> Vec2 {
        let want = target + Vec2::from(self.offset);
        let mut goal = current;
        for axis in 0..2 {
            let d = want[axis] - current[axis];
            let zone = if self.settled {
                self.dead_zone[axis].max(0.0)
            } else {
                0.0
            };
            if d.abs() > zone {
                goal[axis] = want[axis] - zone * d.signum();
            }
        }
        let mut next = if !self.settled || self.smoothing <= 0.0 {
            goal
        } else {
            current.lerp(goal, 1.0 - (-dt / self.smoothing).exp())
        };
        if self.bounded() {
            let half = Vec2::new(half_height * 16.0 / 9.0, half_height);
            let [x0, y0, x1, y1] = self.bounds;
            for (axis, (lo, hi)) in [(x0, x1), (y0, y1)].into_iter().enumerate() {
                let (lo, hi) = (lo + half[axis], hi - half[axis]);
                next[axis] = if lo > hi {
                    f32::midpoint(lo, hi)
                } else {
                    next[axis].clamp(lo, hi)
                };
            }
        }
        next
    }
}

/// A sprite's or tilemap's loaded image (by path and filtering).
#[derive(Debug, Clone)]
pub struct SpriteTexture {
    key: (String, bool),
    /// The texture (white for no image, or one that failed to load).
    pub texture: Texture,
}

/// Loads the images of sprites and tilemaps that don't have theirs yet
/// (shared through the asset cache; called by
/// [`crate::materialize_visuals`]).
pub(crate) fn prepare_sprites(world: &mut SceneWorld, renderer: &Renderer) {
    let mut wanted: Vec<(hecs::Entity, (String, bool))> = Vec::new();
    for (entity, sprite, map, have) in &mut world.query::<(
        hecs::Entity,
        Option<&Sprite2D>,
        Option<&Tilemap>,
        Option<&SpriteTexture>,
    )>() {
        let key = match (sprite, map) {
            (Some(s), _) => (s.image.clone(), s.pixelated),
            (None, Some(m)) => (m.tileset.clone(), m.pixelated),
            (None, None) => continue,
        };
        if have.is_none_or(|h| h.key != key) {
            wanted.push((entity, key));
        }
    }
    for (entity, key) in wanted {
        let texture = if key.0.trim().is_empty() {
            renderer.white_texture()
        } else {
            automata_assets::sprite_texture(renderer, &key.0, key.1).unwrap_or_else(|e| {
                warn_once(&e);
                renderer.white_texture()
            })
        };
        let _ = world.insert_one(entity, SpriteTexture { key, texture });
    }
}

fn warn_once(message: &str) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.iter().any(|s| s == message) {
        tracing::warn!("sprite image {message}");
        seen.push(message.to_string());
    }
}

/// Insets a texture window by `texels` texels on every side (keeps
/// neighbouring sheet frames from bleeding in), keeping flips.
fn inset(uv: [f32; 4], size: (u32, u32), texels: f32) -> [f32; 4] {
    #[allow(clippy::cast_precision_loss)] // texture sizes
    let (du, dv) = (texels / size.0.max(1) as f32, texels / size.1.max(1) as f32);
    let [u0, v0, u1, v1] = uv;
    let su = if u1 >= u0 { du } else { -du };
    let sv = if v1 >= v0 { dv } else { -dv };
    [u0 + su, v0 + sv, u1 - su, v1 - sv]
}

fn texels_for(pixelated: bool) -> f32 {
    if pixelated {
        0.01
    } else {
        0.5
    }
}

/// One world sprite waiting to be sorted.
struct Pending {
    layer: i32,
    z: f32,
    sprite: Sprite,
}

/// Records sprites and tilemaps, sorted by layer then Z (called by
/// [`crate::record_draw_list`]).
pub(crate) fn record_sprites(world: &SceneWorld, draw: &mut DrawList) {
    let mut pending: Vec<Pending> = Vec::new();
    for (sprite, global, texture) in
        &mut world.query::<(&Sprite2D, &GlobalTransform, &SpriteTexture)>()
    {
        let (scale, rotation, translation) = global.matrix().to_scale_rotation_translation();
        let (z_rotation, _, _) = rotation.to_euler(EulerRot::ZYX);
        let tex = &texture.texture;
        let uv = inset(sprite.uv(), tex.size(), texels_for(sprite.pixelated));
        pending.push(Pending {
            layer: sprite.layer,
            z: translation.z,
            sprite: Sprite::new(tex.clone())
                .position(translation.truncate())
                .size(Vec2::from(sprite.size) * scale.truncate())
                .rotation(z_rotation)
                .color(sprite.color)
                .uv(uv),
        });
    }
    for (map, global, texture) in &mut world.query::<(&Tilemap, &GlobalTransform, &SpriteTexture)>()
    {
        record_tilemap(map, global, &texture.texture, &mut pending);
    }
    pending.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.z.total_cmp(&b.z)));
    for p in pending {
        draw.sprite(p.sprite);
    }
}

fn record_tilemap(map: &Tilemap, global: &GlobalTransform, tex: &Texture, out: &mut Vec<Pending>) {
    let m = global.matrix();
    let (scale, rotation, translation) = m.to_scale_rotation_translation();
    let (z_rotation, _, _) = rotation.to_euler(EulerRot::ZYX);
    // A hair larger than the cell so neighbours never show a crack.
    let size = Vec2::splat(map.tile_size) * scale.truncate() * 1.002;
    let texels = texels_for(map.pixelated);
    for y in 0..map.height {
        for x in 0..map.width {
            let Some(tile) = map.tile_at(i64::from(x), i64::from(y)) else {
                continue;
            };
            #[allow(clippy::cast_precision_loss)] // ≤ 1024
            let local = Vec3::new(
                (x as f32 + 0.5) * map.tile_size,
                (y as f32 + 0.5) * map.tile_size,
                0.0,
            );
            let centre = m.transform_point3(local);
            out.push(Pending {
                layer: map.layer,
                z: translation.z,
                sprite: Sprite::new(tex.clone())
                    .position(centre.truncate())
                    .size(size)
                    .rotation(z_rotation)
                    .color(map.color)
                    .uv(inset(map.tile_uv(tile), tex.size(), texels)),
            });
        }
    }
}

/// The scene's 2D camera as drawn: its centre moved by the entity's
/// position.
#[must_use]
pub fn scene_camera_2d(world: &SceneWorld) -> Option<Camera2D> {
    let mut query = world.query::<(&Camera2D, Option<&GlobalTransform>)>();
    let (camera, global) = query.iter().next()?;
    let mut camera = *camera;
    if let Some(g) = global {
        camera.center += g.position().truncate();
    }
    Some(camera)
}

/// A 3D camera seeing what `camera` sees (meshes in a 2D scene line up
/// with its sprites).
#[must_use]
pub fn ortho_camera_for(camera: &Camera2D) -> automata_renderer::Camera3D {
    let c = camera.center;
    automata_renderer::Camera3D {
        ortho: camera.half_height.max(1e-3),
        near: 0.1,
        far: 2000.0,
        ..automata_renderer::Camera3D::new(Vec3::new(c.x, c.y, 500.0), Vec3::new(c.x, c.y, 0.0))
    }
}

/// Plays every sprite's clip forward by `dt` seconds.
pub fn animate_sprites(world: &mut SceneWorld, dt: f32) {
    for sprite in world.query_mut::<&mut Sprite2D>() {
        sprite.advance(dt);
    }
}

/// Moves every following 2D camera toward its target (by name) over `dt`
/// seconds.
pub fn follow_cameras_2d(world: &mut SceneWorld, dt: f32) {
    let targets: Vec<(hecs::Entity, Vec2)> = world
        .query::<(hecs::Entity, &CameraFollow2D, &Camera2D)>()
        .iter()
        .filter_map(|(e, follow, _)| {
            let mut names = world.query::<(
                &Name,
                Option<&GlobalTransform>,
                Option<&Transform>,
                Option<&crate::transform::Parent>,
            )>();
            let (_, global, local, parent) =
                names.iter().find(|(n, _, _, _)| n.0 == follow.target)?;
            // A root entity's own transform is this frame's; a child's
            // world position is last frame's propagation.
            let p = match (local, parent, global) {
                (Some(t), None, _) | (Some(t), _, None) => t.position,
                (_, _, Some(g)) => g.position(),
                (None, _, None) => return None,
            };
            Some((e, p.truncate()))
        })
        .collect();
    for (entity, target) in targets {
        let Ok((follow, camera, transform)) =
            world.query_one_mut::<(&mut CameraFollow2D, &mut Camera2D, Option<&mut Transform>)>(
                entity,
            )
        else {
            continue;
        };
        // The entity's position moves (the editor shows where it went);
        // without a transform, the camera's own centre.
        let base = transform
            .as_ref()
            .map_or(Vec2::ZERO, |t| t.position.truncate());
        let next = follow.step(base + camera.center, target, camera.half_height, dt);
        match transform {
            Some(t) => {
                let moved = next - camera.center;
                t.position.x = moved.x;
                t.position.y = moved.y;
            }
            None => camera.center = next,
        }
        follow.settled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_loop_or_hold_their_last_frame() {
        let mut clip = SpriteClip::new("run", 4, 7, 10.0);
        assert_eq!(clip.frame_at(0.0), (4, false));
        assert_eq!(clip.frame_at(0.35), (7, false));
        assert_eq!(clip.frame_at(0.45), (4, false));
        clip.looping = false;
        assert_eq!(clip.frame_at(0.45), (7, true));
    }

    #[test]
    fn sprites_start_their_clip_and_switch_on_request() {
        let mut s = Sprite2D {
            columns: 4,
            rows: 2,
            clips: vec![
                SpriteClip::new("idle", 0, 1, 4.0),
                SpriteClip::new("run", 4, 7, 10.0),
            ],
            play: "idle".into(),
            ..Sprite2D::default()
        };
        s.advance(0.3);
        assert_eq!((s.playing.as_str(), s.frame), ("idle", 1));
        assert!(s.play_clip("run", false));
        s.advance(0.15);
        assert_eq!(s.frame, 5);
        // Asking again doesn't restart it.
        assert!(s.play_clip("run", false));
        assert_eq!(s.frame, 5);
        assert!(!s.play_clip("swim", false));
        s.flip_x = true;
        let [u0, _, u1, _] = s.uv();
        assert!(u0 > u1, "flipped");
    }

    #[test]
    fn the_camera_snaps_then_eases_through_its_dead_zone() {
        let mut f = CameraFollow2D {
            offset: [0.0, 0.0],
            smoothing: 0.0,
            dead_zone: [1.0, 1.0],
            ..CameraFollow2D::default()
        };
        let first = f.step(Vec2::ZERO, Vec2::new(10.0, 0.0), 5.0, 1.0 / 60.0);
        assert_eq!(first, Vec2::new(10.0, 0.0), "snaps the first frame");
        f.settled = true;
        let inside = f.step(first, Vec2::new(10.5, 0.0), 5.0, 1.0 / 60.0);
        assert_eq!(inside, first, "the dead zone");
        let pushed = f.step(first, Vec2::new(12.0, 0.0), 5.0, 1.0 / 60.0);
        assert_eq!(pushed, Vec2::new(11.0, 0.0));
        f.bounds = [0.0, 0.0, 20.0, 10.0];
        let kept = f.step(first, Vec2::new(40.0, -9.0), 5.0, 1.0 / 60.0);
        let half_w = 5.0 * 16.0 / 9.0;
        assert!((kept.x - (20.0 - half_w)).abs() < 1e-4, "{kept}");
        assert!((kept.y - 5.0).abs() < 1e-4, "{kept}");
    }
}
