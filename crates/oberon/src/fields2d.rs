//! Editable properties of the 2D components: Sprite (and its clips),
//! Tilemap, Camera 2D, Follow Camera 2D and Collider 2D.
//!
//! They hang off [`crate::fields`] as [`Field::TwoD`] and [`Flag::TwoD`]
//! (and [`Choice::Collider2DShape`]), so the Inspector, scrubbing and typed
//! edits treat them like every other number.
//!
//! [`Field::TwoD`]: crate::fields::Field::TwoD
//! [`Flag::TwoD`]: crate::fields::Flag::TwoD
//! [`Choice::Collider2DShape`]: crate::fields::Choice::Collider2DShape

#![forbid(unsafe_code)]

use automata_core::EntityData;
use automata_physics::Shape2D;

/// One number of a 2D component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field2D {
    /// Sprite width (0) / height (1), world units.
    SpriteSize(u8),
    /// Sprite tint channel 0..4 (RGBA).
    SpriteColor(u8),
    /// Draw order: higher layers draw over lower ones.
    SpriteLayer,
    /// Sprite sheet frames across.
    SpriteColumns,
    /// Sprite sheet frames down.
    SpriteRows,
    /// The frame shown while no clip plays.
    SpriteFrame,
    /// A clip's first frame (clip index).
    ClipFirst(u8),
    /// A clip's last frame (clip index).
    ClipLast(u8),
    /// A clip's frames per second (clip index).
    ClipFps(u8),
    /// Tilemap cells across.
    TileWidth,
    /// Tilemap cells up.
    TileHeight,
    /// A cell's side, world units.
    TileSize,
    /// Tileset tiles across.
    TileColumns,
    /// Tileset tiles down.
    TileRows,
    /// Tilemap draw order.
    TileLayer,
    /// Tilemap tint channel 0..4 (RGBA).
    TileColor(u8),
    /// An auto-tile's first tile (auto-tile index): its 16 tiles follow.
    AutoFirst(u8),
    /// Camera 2D: half the height it sees, world units.
    CameraHalfHeight,
    /// Follow camera: seconds to catch up (0 = locked on).
    FollowSmoothing,
    /// Follow camera: where it looks from the target, X (0) / Y (1).
    FollowOffset(u8),
    /// Follow camera: how far the target moves before it follows, X / Y.
    FollowDeadZone(u8),
    /// Follow camera: what it may show, left, bottom, right, top (all 0 =
    /// anywhere).
    FollowBounds(u8),
    /// Collider 2D box width (0) / height (1).
    ColliderExtents(u8),
    /// Collider 2D circle / capsule radius.
    ColliderRadius,
    /// Collider 2D capsule: half the straight part.
    ColliderHalfHeight,
    /// Collider 2D density.
    ColliderDensity,
    /// Collider 2D friction.
    ColliderFriction,
    /// Collider 2D bounciness.
    ColliderRestitution,
}

/// A checkbox of a 2D component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag2D {
    /// The sprite is mirrored left to right.
    SpriteFlipX,
    /// The sprite is mirrored top to bottom.
    SpriteFlipY,
    /// The sprite keeps hard pixel edges (pixel art).
    SpritePixelated,
    /// A clip starts over when it ends (clip index).
    ClipLoop(u8),
    /// The tiles keep hard pixel edges.
    TilePixelated,
    /// Painted cells are solid.
    TileCollide,
    /// The 2D collider is a trigger.
    ColliderSensor,
    /// The 2D collider collides.
    ColliderEnabled,
}

/// Whether a field holds a whole number.
#[must_use]
pub fn whole(field: Field2D) -> bool {
    use Field2D as F;
    matches!(
        field,
        F::SpriteLayer
            | F::SpriteColumns
            | F::SpriteRows
            | F::SpriteFrame
            | F::ClipFirst(_)
            | F::ClipLast(_)
            | F::TileWidth
            | F::TileHeight
            | F::TileColumns
            | F::TileRows
            | F::TileLayer
            | F::AutoFirst(_)
    )
}

/// The range a field is kept in.
#[must_use]
pub fn range(field: Field2D) -> (f32, f32) {
    use Field2D as F;
    match field {
        F::SpriteColor(_) | F::TileColor(_) | F::ColliderRestitution => (0.0, 1.0),
        F::SpriteSize(_) | F::ColliderExtents(_) => (0.01, 10_000.0),
        F::SpriteLayer | F::TileLayer => (-1_000.0, 1_000.0),
        F::SpriteColumns | F::SpriteRows | F::TileColumns | F::TileRows => (1.0, 256.0),
        F::SpriteFrame | F::ClipFirst(_) | F::ClipLast(_) | F::AutoFirst(_) => (0.0, 65_535.0),
        F::ClipFps(_) => (0.1, 120.0),
        F::TileWidth | F::TileHeight => (1.0, 1_024.0),
        F::TileSize => (0.01, 100.0),
        F::CameraHalfHeight => (0.1, 10_000.0),
        F::FollowSmoothing => (0.0, 5.0),
        F::FollowDeadZone(_) => (0.0, 1_000.0),
        F::ColliderRadius => (0.005, 10_000.0),
        F::ColliderHalfHeight | F::ColliderDensity | F::ColliderFriction => (0.0, 10_000.0),
        F::FollowOffset(_) | F::FollowBounds(_) => (-1_000_000.0, 1_000_000.0),
    }
}

/// Value change per pixel when scrubbing.
#[must_use]
pub fn scrub_step(field: Field2D) -> f32 {
    use Field2D as F;
    if whole(field) {
        return 0.1;
    }
    match field {
        F::SpriteColor(_) | F::TileColor(_) | F::ColliderRestitution | F::FollowSmoothing => 0.005,
        F::ClipFps(_) => 0.05,
        _ => 0.01,
    }
}

fn slot(i: u8, n: usize) -> usize {
    usize::from(i).min(n - 1)
}

/// Reads a field, or `None` if the entity lacks it.
#[must_use]
#[allow(clippy::cast_precision_loss)] // counts and frames well inside f32
pub fn get(data: &EntityData, field: Field2D) -> Option<f32> {
    use Field2D as F;
    Some(match field {
        F::SpriteSize(i) => data.sprite.as_ref()?.size[slot(i, 2)],
        F::SpriteColor(i) => data.sprite.as_ref()?.color[slot(i, 4)],
        F::SpriteLayer => data.sprite.as_ref()?.layer as f32,
        F::SpriteColumns => data.sprite.as_ref()?.columns as f32,
        F::SpriteRows => data.sprite.as_ref()?.rows as f32,
        F::SpriteFrame => data.sprite.as_ref()?.frame as f32,
        F::ClipFirst(i) => data.sprite.as_ref()?.clips.get(usize::from(i))?.first as f32,
        F::ClipLast(i) => data.sprite.as_ref()?.clips.get(usize::from(i))?.last as f32,
        F::ClipFps(i) => data.sprite.as_ref()?.clips.get(usize::from(i))?.fps,
        F::TileWidth => data.tilemap.as_ref()?.width as f32,
        F::TileHeight => data.tilemap.as_ref()?.height as f32,
        F::TileSize => data.tilemap.as_ref()?.tile_size,
        F::TileColumns => data.tilemap.as_ref()?.columns as f32,
        F::TileRows => data.tilemap.as_ref()?.rows as f32,
        F::TileLayer => data.tilemap.as_ref()?.layer as f32,
        F::TileColor(i) => data.tilemap.as_ref()?.color[slot(i, 4)],
        F::AutoFirst(i) => data.tilemap.as_ref()?.autotiles.get(usize::from(i))?.first as f32,
        F::CameraHalfHeight => data.camera_2d?.half_height,
        F::FollowSmoothing => data.camera_follow_2d.as_ref()?.smoothing,
        F::FollowOffset(i) => data.camera_follow_2d.as_ref()?.offset[slot(i, 2)],
        F::FollowDeadZone(i) => data.camera_follow_2d.as_ref()?.dead_zone[slot(i, 2)],
        F::FollowBounds(i) => data.camera_follow_2d.as_ref()?.bounds[slot(i, 4)],
        F::ColliderExtents(i) => match data.collider_2d.as_ref()?.shape {
            Shape2D::Cuboid { extents } => extents[slot(i, 2)],
            _ => return None,
        },
        F::ColliderRadius => match data.collider_2d.as_ref()?.shape {
            Shape2D::Ball { radius } | Shape2D::Capsule { radius, .. } => radius,
            Shape2D::Cuboid { .. } => return None,
        },
        F::ColliderHalfHeight => match data.collider_2d.as_ref()?.shape {
            Shape2D::Capsule { half_height, .. } => half_height,
            _ => return None,
        },
        F::ColliderDensity => data.collider_2d.as_ref()?.density,
        F::ColliderFriction => data.collider_2d.as_ref()?.friction,
        F::ColliderRestitution => data.collider_2d.as_ref()?.restitution,
    })
}

/// Writes a field (already clamped to its range; whole fields are
/// rounded). Returns whether the entity has it.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped first
pub fn set(data: &mut EntityData, field: Field2D, v: f32) -> bool {
    use Field2D as F;
    let v = if whole(field) { v.round() } else { v };
    let n = v as i64;
    let u = n.max(0) as u32;
    match field {
        F::SpriteSize(_)
        | F::SpriteColor(_)
        | F::SpriteLayer
        | F::SpriteColumns
        | F::SpriteRows
        | F::SpriteFrame => {
            let Some(s) = &mut data.sprite else {
                return false;
            };
            match field {
                F::SpriteSize(i) => s.size[slot(i, 2)] = v,
                F::SpriteColor(i) => s.color[slot(i, 4)] = v,
                F::SpriteLayer => s.layer = n as i32,
                F::SpriteColumns => s.columns = u,
                F::SpriteRows => s.rows = u,
                _ => s.frame = u,
            }
        }
        F::ClipFirst(i) | F::ClipLast(i) | F::ClipFps(i) => {
            let Some(clip) = data
                .sprite
                .as_mut()
                .and_then(|s| s.clips.get_mut(usize::from(i)))
            else {
                return false;
            };
            match field {
                F::ClipFirst(_) => clip.first = u,
                F::ClipLast(_) => clip.last = u,
                _ => clip.fps = v,
            }
        }
        F::TileWidth
        | F::TileHeight
        | F::TileSize
        | F::TileColumns
        | F::TileRows
        | F::TileLayer
        | F::TileColor(_) => {
            let Some(m) = &mut data.tilemap else {
                return false;
            };
            match field {
                // Resizing keeps what's painted, anchored bottom-left.
                F::TileWidth => m.resize(u, m.height),
                F::TileHeight => m.resize(m.width, u),
                F::TileSize => m.tile_size = v,
                F::TileColumns => m.columns = u,
                F::TileRows => m.rows = u,
                F::TileLayer => m.layer = n as i32,
                F::TileColor(i) => m.color[slot(i, 4)] = v,
                _ => {}
            }
        }
        F::AutoFirst(i) => {
            let Some(a) = data
                .tilemap
                .as_mut()
                .and_then(|m| m.autotiles.get_mut(usize::from(i)))
            else {
                return false;
            };
            a.first = u;
        }
        F::CameraHalfHeight => match &mut data.camera_2d {
            Some(c) => c.half_height = v,
            None => return false,
        },
        F::FollowSmoothing | F::FollowOffset(_) | F::FollowDeadZone(_) | F::FollowBounds(_) => {
            let Some(f) = &mut data.camera_follow_2d else {
                return false;
            };
            match field {
                F::FollowSmoothing => f.smoothing = v,
                F::FollowOffset(i) => f.offset[slot(i, 2)] = v,
                F::FollowDeadZone(i) => f.dead_zone[slot(i, 2)] = v,
                F::FollowBounds(i) => f.bounds[slot(i, 4)] = v,
                _ => {}
            }
        }
        F::ColliderExtents(_)
        | F::ColliderRadius
        | F::ColliderHalfHeight
        | F::ColliderDensity
        | F::ColliderFriction
        | F::ColliderRestitution => {
            let Some(c) = &mut data.collider_2d else {
                return false;
            };
            match (field, &mut c.shape) {
                (F::ColliderExtents(i), Shape2D::Cuboid { extents }) => {
                    extents[slot(i, 2)] = v;
                }
                (F::ColliderRadius, Shape2D::Ball { radius } | Shape2D::Capsule { radius, .. }) => {
                    *radius = v;
                }
                (F::ColliderHalfHeight, Shape2D::Capsule { half_height, .. }) => *half_height = v,
                (F::ColliderDensity, _) => c.density = v,
                (F::ColliderFriction, _) => c.friction = v,
                (F::ColliderRestitution, _) => c.restitution = v,
                _ => return false,
            }
        }
    }
    true
}

/// Reads a checkbox, or `None` if the entity lacks it.
#[must_use]
pub fn get_flag(data: &EntityData, flag: Flag2D) -> Option<bool> {
    Some(match flag {
        Flag2D::SpriteFlipX => data.sprite.as_ref()?.flip_x,
        Flag2D::SpriteFlipY => data.sprite.as_ref()?.flip_y,
        Flag2D::SpritePixelated => data.sprite.as_ref()?.pixelated,
        Flag2D::ClipLoop(i) => data.sprite.as_ref()?.clips.get(usize::from(i))?.looping,
        Flag2D::TilePixelated => data.tilemap.as_ref()?.pixelated,
        Flag2D::TileCollide => data.tilemap.as_ref()?.collide,
        Flag2D::ColliderSensor => data.collider_2d.as_ref()?.sensor,
        Flag2D::ColliderEnabled => data.collider_2d.as_ref()?.enabled,
    })
}

/// Flips a checkbox. Returns whether the entity has it.
pub fn toggle(data: &mut EntityData, flag: Flag2D) -> bool {
    let slot: Option<&mut bool> = match flag {
        Flag2D::SpriteFlipX => data.sprite.as_mut().map(|s| &mut s.flip_x),
        Flag2D::SpriteFlipY => data.sprite.as_mut().map(|s| &mut s.flip_y),
        Flag2D::SpritePixelated => data.sprite.as_mut().map(|s| &mut s.pixelated),
        Flag2D::ClipLoop(i) => data
            .sprite
            .as_mut()
            .and_then(|s| s.clips.get_mut(usize::from(i)))
            .map(|c| &mut c.looping),
        Flag2D::TilePixelated => data.tilemap.as_mut().map(|m| &mut m.pixelated),
        Flag2D::TileCollide => data.tilemap.as_mut().map(|m| &mut m.collide),
        Flag2D::ColliderSensor => data.collider_2d.as_mut().map(|c| &mut c.sensor),
        Flag2D::ColliderEnabled => data.collider_2d.as_mut().map(|c| &mut c.enabled),
    };
    match slot {
        Some(b) => {
            *b = !*b;
            true
        }
        None => false,
    }
}

/// The 2D collider's shape name.
#[must_use]
pub fn shape_label(data: &EntityData) -> Option<&'static str> {
    Some(match data.collider_2d.as_ref()?.shape {
        Shape2D::Cuboid { .. } => "Box",
        Shape2D::Ball { .. } => "Circle",
        Shape2D::Capsule { .. } => "Capsule",
    })
}

/// Box → Circle → Capsule → Box, keeping about the same size.
pub fn cycle_shape(data: &mut EntityData) -> bool {
    let Some(c) = &mut data.collider_2d else {
        return false;
    };
    c.shape = match c.shape {
        Shape2D::Cuboid { extents } => Shape2D::Ball {
            radius: extents.max_element() * 0.5,
        },
        Shape2D::Ball { radius } => Shape2D::Capsule {
            half_height: radius * 0.5,
            radius: radius * 0.5,
        },
        Shape2D::Capsule {
            half_height,
            radius,
        } => Shape2D::Cuboid {
            extents: glam::Vec2::new(radius * 2.0, (half_height + radius) * 2.0),
        },
    };
    true
}

/// A 2D component edit that isn't a number or a checkbox: pictures,
/// names, and adding or removing clips and auto-tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit2D {
    /// The sprite's image (a path from the scene folder; "" = a plain
    /// rectangle).
    SpriteImage(String),
    /// The clip the sprite starts playing ("" = none: it shows its frame).
    SpritePlay(String),
    /// Adds a clip ("idle", "run" …: the next free name) over every frame.
    AddClip,
    /// Removes clip `n`.
    RemoveClip(usize),
    /// Renames clip `n` (the starting clip follows).
    RenameClip(usize, String),
    /// The tilemap's tileset image ("" = plain squares).
    Tileset(String),
    /// Adds an auto-tile after the plain tiles.
    AddAutoTile,
    /// Removes auto-tile `n`; its cells are emptied, later ones renumbered.
    RemoveAutoTile(usize),
    /// Empties every cell.
    ClearTiles,
    /// The entity the 2D camera follows, by name.
    FollowTarget(String),
}

/// Clip names offered in turn by [`Edit2D::AddClip`].
const CLIP_NAMES: [&str; 6] = ["idle", "run", "jump", "fall", "attack", "hurt"];

impl Edit2D {
    /// Applies the edit; returns whether the entity has what it edits.
    pub fn apply(&self, data: &mut EntityData) -> bool {
        match self {
            Self::SpriteImage(_)
            | Self::SpritePlay(_)
            | Self::AddClip
            | Self::RemoveClip(_)
            | Self::RenameClip(..) => {
                let Some(s) = &mut data.sprite else {
                    return false;
                };
                match self {
                    Self::SpriteImage(path) => s.image = path.replace('\\', "/"),
                    Self::SpritePlay(name) => s.play.clone_from(name),
                    Self::AddClip => {
                        let taken = |n: &str| s.clips.iter().any(|c| c.name == n);
                        let name = CLIP_NAMES
                            .iter()
                            .map(|n| (*n).to_string())
                            .chain((1..=s.clips.len() + 1).map(|i| format!("clip{i}")))
                            .find(|n| !taken(n))
                            .unwrap_or_default();
                        let last = (s.columns.max(1) * s.rows.max(1)).saturating_sub(1);
                        s.clips
                            .push(automata_scene::SpriteClip::new(&name, 0, last, 10.0));
                        if s.play.is_empty() && s.clips.len() == 1 {
                            s.play = name;
                        }
                    }
                    Self::RemoveClip(n) => {
                        if *n >= s.clips.len() {
                            return false;
                        }
                        let gone = s.clips.remove(*n);
                        if s.play == gone.name {
                            s.play.clear();
                        }
                    }
                    Self::RenameClip(n, name) => {
                        let name = name.trim();
                        if name.is_empty() || s.clips.iter().any(|c| c.name == name) {
                            return false;
                        }
                        let Some(clip) = s.clips.get_mut(*n) else {
                            return false;
                        };
                        if s.play == clip.name {
                            s.play = name.to_string();
                        }
                        clip.name = name.to_string();
                    }
                    _ => {}
                }
            }
            Self::Tileset(_) | Self::AddAutoTile | Self::RemoveAutoTile(_) | Self::ClearTiles => {
                let Some(m) = &mut data.tilemap else {
                    return false;
                };
                match self {
                    Self::Tileset(path) => m.tileset = path.replace('\\', "/"),
                    Self::AddAutoTile => {
                        let n = m.autotiles.len() + 1;
                        m.autotiles.push(automata_scene::AutoTile {
                            name: format!("auto{n}"),
                            first: 0,
                        });
                    }
                    Self::RemoveAutoTile(n) => {
                        if *n >= m.autotiles.len() {
                            return false;
                        }
                        m.autotiles.remove(*n);
                        // Cell -k-1 is auto-tile k: empty the removed
                        // one's cells and move later ones down.
                        let gone = -(i32::try_from(*n).unwrap_or(i32::MAX)) - 1;
                        for c in &mut m.cells {
                            if *c == gone {
                                *c = 0;
                            } else if *c < gone {
                                *c += 1;
                            }
                        }
                    }
                    Self::ClearTiles => m.clear(),
                    _ => {}
                }
            }
            Self::FollowTarget(name) => match &mut data.camera_follow_2d {
                Some(f) => f.target.clone_from(name),
                None => return false,
            },
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::ComponentKind;

    #[test]
    fn sprite_and_tilemap_numbers_round_and_resize() {
        let mut d = EntityData::default();
        assert_eq!(get(&d, Field2D::SpriteColumns), None);
        ComponentKind::Sprite.add_to(&mut d);
        ComponentKind::Tilemap.add_to(&mut d);
        assert!(set(&mut d, Field2D::SpriteColumns, 3.6));
        assert_eq!(d.sprite.as_ref().unwrap().columns, 4);
        assert!(set(&mut d, Field2D::SpriteSize(1), 2.5));
        assert_eq!(d.sprite.as_ref().unwrap().size, [1.0, 2.5]);
        assert!(!set(&mut d, Field2D::ClipFps(0), 8.0), "no clip yet");
        d.sprite
            .as_mut()
            .unwrap()
            .clips
            .push(automata_scene::SpriteClip::new("run", 0, 3, 10.0));
        assert!(set(&mut d, Field2D::ClipLast(0), 5.0));
        assert_eq!(get(&d, Field2D::ClipLast(0)), Some(5.0));
        assert!(toggle(&mut d, Flag2D::ClipLoop(0)));
        assert_eq!(get_flag(&d, Flag2D::ClipLoop(0)), Some(false));

        d.tilemap.as_mut().unwrap().set(0, 0, 2);
        assert!(set(&mut d, Field2D::TileWidth, 4.0));
        let m = d.tilemap.as_ref().unwrap();
        assert_eq!(
            (m.width, m.height, m.get(0, 0)),
            (4, 9, 2),
            "kept what's painted"
        );
    }

    #[test]
    fn collider_shapes_cycle_at_about_the_same_size() {
        let mut d = EntityData::default();
        ComponentKind::Collider2D.add_to(&mut d);
        assert_eq!(shape_label(&d), Some("Box"));
        assert_eq!(get(&d, Field2D::ColliderRadius), None);
        cycle_shape(&mut d);
        assert_eq!(shape_label(&d), Some("Circle"));
        assert_eq!(get(&d, Field2D::ColliderRadius), Some(0.5));
        cycle_shape(&mut d);
        assert_eq!(shape_label(&d), Some("Capsule"));
        cycle_shape(&mut d);
        assert_eq!(get(&d, Field2D::ColliderExtents(1)), Some(1.0));
    }

    #[test]
    fn clips_are_added_named_renamed_and_removed() {
        let mut d = EntityData::default();
        ComponentKind::Sprite.add_to(&mut d);
        set(&mut d, Field2D::SpriteColumns, 4.0);
        assert!(Edit2D::AddClip.apply(&mut d));
        assert!(Edit2D::AddClip.apply(&mut d));
        let s = d.sprite.as_ref().unwrap();
        let names: Vec<_> = s.clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["idle", "run"]);
        assert_eq!(s.play, "idle", "the first clip plays from the start");
        assert_eq!(
            (s.clips[0].first, s.clips[0].last),
            (0, 3),
            "over every frame"
        );
        assert!(!Edit2D::RenameClip(1, "idle".into()).apply(&mut d), "taken");
        assert!(Edit2D::RenameClip(0, "stand".into()).apply(&mut d));
        assert_eq!(d.sprite.as_ref().unwrap().play, "stand");
        assert!(Edit2D::RemoveClip(0).apply(&mut d));
        let s = d.sprite.as_ref().unwrap();
        assert_eq!((s.clips.len(), s.play.as_str()), (1, ""));
    }

    #[test]
    fn removing_an_auto_tile_renumbers_the_cells() {
        let mut d = EntityData::default();
        ComponentKind::Tilemap.add_to(&mut d);
        Edit2D::AddAutoTile.apply(&mut d);
        Edit2D::AddAutoTile.apply(&mut d);
        let m = d.tilemap.as_mut().unwrap();
        m.set(0, 0, -1);
        m.set(1, 0, -2);
        m.set(2, 0, 3);
        assert!(Edit2D::RemoveAutoTile(0).apply(&mut d));
        let m = d.tilemap.as_ref().unwrap();
        assert_eq!((m.get(0, 0), m.get(1, 0), m.get(2, 0)), (0, -1, 3));
        assert_eq!(m.autotiles.len(), 1);
    }
}
