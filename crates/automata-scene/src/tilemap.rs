//! Tilemaps: a grid of tiles cut from one tileset image, painted cell by
//! cell, drawn as 2D sprites and (optionally) solid for 2D physics.
//!
//! The grid starts at the entity's position (the bottom-left corner of
//! cell `(0, 0)`) and grows right (+X) and up (+Y). A cell holds:
//!
//! - `0`: empty;
//! - `n > 0`: tile `n - 1` of the tileset, counting left to right, top to
//!   bottom;
//! - `n < 0`: auto-tile `-n - 1` ([`AutoTile`]): the tile is picked from
//!   which neighbours hold the same auto-tile, so painted ground gets its
//!   edges and corners by itself.

use serde::{Deserialize, Serialize};

/// The most cells a tilemap side may have.
pub const MAX_TILEMAP_SIDE: u32 = 1024;

/// A set of 16 tiles painted as one: each cell picks its tile from which of
/// its four neighbours (up, right, down, left) hold the same auto-tile.
///
/// The 16 tiles follow each other in the tileset from `first`, ordered by
/// the neighbour mask `up·1 + right·2 + down·4 + left·8`: tile `first + 0`
/// stands alone, `first + 5` is a vertical middle (up and down), `first +
/// 14` a top edge (right, down and left), `first + 15` all surrounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoTile {
    /// Shown on the painter's brush.
    pub name: String,
    /// The tileset index of the first of the 16 tiles.
    pub first: u32,
}

/// A grid of tiles from one tileset image (see the module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "TilemapFile", into = "TilemapFile")]
pub struct Tilemap {
    /// The tileset image (relative to the scene, like model paths).
    pub tileset: String,
    /// The tileset's tiles across.
    pub columns: u32,
    /// The tileset's tiles down.
    pub rows: u32,
    /// A cell's side in world units (before the entity's scale).
    pub tile_size: f32,
    /// Cells across.
    pub width: u32,
    /// Cells up.
    pub height: u32,
    /// The cells, `width × height`, row by row from the bottom (see the
    /// module docs). Saved as one line of numbers per row, top row first.
    pub cells: Vec<i32>,
    /// Auto-tiles: `-1` in a cell is the first, `-2` the second, …
    pub autotiles: Vec<AutoTile>,
    /// Tint multiplied with the tileset.
    pub color: [f32; 4],
    /// Hard pixel edges (pixel art) instead of smooth filtering.
    pub pixelated: bool,
    /// Drawing order among sprites: higher layers draw over lower ones.
    pub layer: i32,
    /// Filled cells are solid for 2D physics.
    pub collide: bool,
}

impl Default for Tilemap {
    fn default() -> Self {
        Self::new(16, 9)
    }
}

impl Tilemap {
    /// An empty `width × height` map of 1-unit cells, solid, with a 1×1
    /// tileset (set `tileset`, `columns` and `rows`).
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let (width, height) = (
            width.clamp(1, MAX_TILEMAP_SIDE),
            height.clamp(1, MAX_TILEMAP_SIDE),
        );
        Self {
            tileset: String::new(),
            columns: 1,
            rows: 1,
            tile_size: 1.0,
            width,
            height,
            cells: vec![0; (width * height) as usize],
            autotiles: Vec::new(),
            color: [1.0; 4],
            pixelated: true,
            layer: 0,
            collide: true,
        }
    }

    /// Tiles in the tileset (`columns × rows`).
    #[must_use]
    pub fn tile_count(&self) -> u32 {
        self.columns.max(1) * self.rows.max(1)
    }

    fn index(&self, x: i64, y: i64) -> Option<usize> {
        (x >= 0 && y >= 0 && x < i64::from(self.width) && y < i64::from(self.height)).then(|| {
            // In range: both fit usize.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = (y * i64::from(self.width) + x) as usize;
            i
        })
    }

    /// The cell at `(x, y)`; 0 outside the map.
    #[must_use]
    pub fn get(&self, x: i64, y: i64) -> i32 {
        self.index(x, y)
            .and_then(|i| self.cells.get(i).copied())
            .unwrap_or(0)
    }

    /// Sets the cell at `(x, y)`; returns whether it changed (false outside
    /// the map).
    pub fn set(&mut self, x: i64, y: i64, value: i32) -> bool {
        self.fix_len();
        match self.index(x, y) {
            Some(i) if self.cells[i] != value => {
                self.cells[i] = value;
                true
            }
            _ => false,
        }
    }

    /// Sets every cell from `(x0, y0)` to `(x1, y1)` (inclusive, either
    /// order); returns how many changed.
    pub fn fill(&mut self, from: (i64, i64), to: (i64, i64), value: i32) -> usize {
        let mut changed = 0;
        for y in from.1.min(to.1)..=from.1.max(to.1) {
            for x in from.0.min(to.0)..=from.0.max(to.0) {
                changed += usize::from(self.set(x, y, value));
            }
        }
        changed
    }

    /// Empties every cell.
    pub fn clear(&mut self) {
        self.cells = vec![0; self.len()];
    }

    fn len(&self) -> usize {
        (self.width.max(1) * self.height.max(1)) as usize
    }

    /// Pads or trims `cells` to `width × height` (after hand-editing).
    pub fn fix_len(&mut self) {
        self.width = self.width.clamp(1, MAX_TILEMAP_SIDE);
        self.height = self.height.clamp(1, MAX_TILEMAP_SIDE);
        let len = self.len();
        self.cells.resize(len, 0);
    }

    /// Changes the size, keeping what's painted anchored at the
    /// bottom-left.
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (
            width.clamp(1, MAX_TILEMAP_SIDE),
            height.clamp(1, MAX_TILEMAP_SIDE),
        );
        let mut cells = vec![0; (width * height) as usize];
        for y in 0..height.min(self.height) {
            for x in 0..width.min(self.width) {
                cells[(y * width + x) as usize] = self.get(i64::from(x), i64::from(y));
            }
        }
        self.width = width;
        self.height = height;
        self.cells = cells;
    }

    /// The tileset tile cell `(x, y)` shows (auto-tiles resolved), if any.
    #[must_use]
    pub fn tile_at(&self, x: i64, y: i64) -> Option<u32> {
        let v = self.get(x, y);
        match v {
            0 => None,
            v if v > 0 => Some(v.unsigned_abs() - 1),
            v => {
                let auto = self.autotiles.get((v.unsigned_abs() - 1) as usize)?;
                let same = |dx: i64, dy: i64| u32::from(self.get(x + dx, y + dy) == v);
                let mask = same(0, 1) | same(1, 0) << 1 | same(0, -1) << 2 | same(-1, 0) << 3;
                Some(auto.first + mask)
            }
        }
    }

    /// The cell holding local point `(x, y)` (in the map's units, before
    /// the entity's transform), even outside the map.
    #[must_use]
    pub fn cell_at(&self, x: f32, y: f32) -> (i64, i64) {
        let size = self.tile_size.max(1e-4);
        // Floors of finite values well inside i64.
        #[allow(clippy::cast_possible_truncation)]
        ((x / size).floor() as i64, (y / size).floor() as i64)
    }

    /// The map's size in its units: `(width, height) × tile_size`.
    #[must_use]
    pub fn extent(&self) -> [f32; 2] {
        #[allow(clippy::cast_precision_loss)] // ≤ 1024
        [
            self.width as f32 * self.tile_size,
            self.height as f32 * self.tile_size,
        ]
    }

    /// The tileset's texture window `(u0, v0, u1, v1)` for `tile`.
    #[must_use]
    pub fn tile_uv(&self, tile: u32) -> [f32; 4] {
        frame_uv(tile, self.columns, self.rows)
    }

    /// Solid cells merged into as few boxes as possible, each
    /// `[x0, y0, x1, y1]` in cells (end exclusive): rows are run together
    /// and runs of equal width stacked. Physics makes one box collider
    /// each, so the ground has no seams to snag on.
    #[must_use]
    pub fn solid_boxes(&self) -> Vec<[u32; 4]> {
        let (w, h) = (self.width, self.height);
        let mut used = vec![false; (w * h) as usize];
        let solid = |x: u32, y: u32, used: &[bool]| {
            self.get(i64::from(x), i64::from(y)) != 0 && !used[(y * w + x) as usize]
        };
        let mut boxes = Vec::new();
        for y in 0..h {
            let mut x = 0;
            while x < w {
                if !solid(x, y, &used) {
                    x += 1;
                    continue;
                }
                let x0 = x;
                while x < w && solid(x, y, &used) {
                    x += 1;
                }
                // Grow the run upward while the whole row above is free.
                let mut y1 = y + 1;
                while y1 < h && (x0..x).all(|cx| solid(cx, y1, &used)) {
                    y1 += 1;
                }
                for cy in y..y1 {
                    for cx in x0..x {
                        used[(cy * w + cx) as usize] = true;
                    }
                }
                boxes.push([x0, y, x, y1]);
            }
        }
        boxes
    }

    /// A content hash (physics rebuilds its colliders when it changes).
    #[must_use]
    pub fn shape_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.width.hash(&mut h);
        self.height.hash(&mut h);
        self.tile_size.to_bits().hash(&mut h);
        self.collide.hash(&mut h);
        for &c in &self.cells {
            (c != 0).hash(&mut h);
        }
        h.finish()
    }

    /// A map drawn as text, top row first: each character is looked up in
    /// `legend` (missing characters, spaces and `.` are empty).
    #[must_use]
    pub fn from_rows(rows: &[&str], legend: &[(char, i32)]) -> Self {
        let height = u32::try_from(rows.len()).unwrap_or(1).max(1);
        let width = rows
            .iter()
            .map(|r| u32::try_from(r.chars().count()).unwrap_or(1))
            .max()
            .unwrap_or(1)
            .max(1);
        let mut map = Self::new(width, height);
        for (i, row) in rows.iter().enumerate() {
            let y = i64::from(map.height) - 1 - i64::try_from(i).unwrap_or(0);
            for (x, c) in row.chars().enumerate() {
                if let Some(&(_, v)) = legend.iter().find(|(k, _)| *k == c) {
                    map.set(i64::try_from(x).unwrap_or(0), y, v);
                }
            }
        }
        map
    }
}

/// The texture window `(u0, v0, u1, v1)` of frame `index` in a sheet of
/// `columns × rows` equal cells, counting left to right, top to bottom
/// (frames past the end wrap).
#[must_use]
pub fn frame_uv(index: u32, columns: u32, rows: u32) -> [f32; 4] {
    let (columns, rows) = (columns.max(1), rows.max(1));
    let index = index % (columns * rows);
    let (c, r) = (index % columns, index / columns);
    #[allow(clippy::cast_precision_loss)] // small counts
    let (cw, ch) = (1.0 / columns as f32, 1.0 / rows as f32);
    #[allow(clippy::cast_precision_loss)]
    let (u0, v0) = (c as f32 * cw, r as f32 * ch);
    [u0, v0, u0 + cw, v0 + ch]
}

/// How a tilemap is saved: the cells as one line of numbers per row, top
/// row first (diffable, hand-editable). Loading also takes a flat list.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct TilemapFile {
    tileset: String,
    columns: u32,
    rows: u32,
    tile_size: f32,
    width: u32,
    height: u32,
    cells: Cells,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    autotiles: Vec<AutoTile>,
    color: [f32; 4],
    pixelated: bool,
    layer: i32,
    collide: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Cells {
    Rows(Vec<String>),
    Flat(Vec<i32>),
}

impl Default for TilemapFile {
    fn default() -> Self {
        Tilemap::default().into()
    }
}

impl From<Tilemap> for TilemapFile {
    fn from(m: Tilemap) -> Self {
        let width = m.width.max(1) as usize;
        let mut rows: Vec<String> = m
            .cells
            .chunks(width)
            .map(|row| row.iter().map(i32::to_string).collect::<Vec<_>>().join(" "))
            .collect();
        rows.reverse();
        Self {
            tileset: m.tileset,
            columns: m.columns,
            rows: m.rows,
            tile_size: m.tile_size,
            width: m.width,
            height: m.height,
            cells: Cells::Rows(rows),
            autotiles: m.autotiles,
            color: m.color,
            pixelated: m.pixelated,
            layer: m.layer,
            collide: m.collide,
        }
    }
}

impl From<TilemapFile> for Tilemap {
    fn from(f: TilemapFile) -> Self {
        let cells = match f.cells {
            Cells::Flat(v) => v,
            Cells::Rows(rows) => rows
                .iter()
                .rev()
                .flat_map(|row| {
                    row.split_whitespace()
                        .map(|n| n.parse::<i32>().unwrap_or(0))
                        .collect::<Vec<_>>()
                })
                .collect(),
        };
        let mut map = Self {
            tileset: f.tileset,
            columns: f.columns.max(1),
            rows: f.rows.max(1),
            tile_size: if f.tile_size > 0.0 { f.tile_size } else { 1.0 },
            width: f.width,
            height: f.height,
            cells,
            autotiles: f.autotiles,
            color: f.color,
            pixelated: f.pixelated,
            layer: f.layer,
            collide: f.collide,
        };
        map.fix_len();
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_tiles_pick_their_edges_from_the_neighbours() {
        let mut map = Tilemap::from_rows(&["....", ".##.", "####"], &[('#', -1)]);
        map.autotiles.push(AutoTile {
            name: "Ground".into(),
            first: 16,
        });
        // Bottom-left: right and up neighbours (up=1, right=2).
        assert_eq!(map.tile_at(0, 0), Some(16 + 2));
        // (1, 0): up, right, left → 1 + 2 + 8.
        assert_eq!(map.tile_at(1, 0), Some(16 + 11));
        // (1, 1): right and down → 2 + 4.
        assert_eq!(map.tile_at(1, 1), Some(16 + 6));
        assert_eq!(map.tile_at(0, 2), None);
        assert_eq!(map.get(-1, 0), 0);
    }

    #[test]
    fn solid_cells_merge_into_few_boxes() {
        let map = Tilemap::from_rows(&["##..", "##..", "####"], &[('#', 1)]);
        let boxes = map.solid_boxes();
        assert_eq!(boxes, vec![[0, 0, 4, 1], [0, 1, 2, 3]]);
        let cells: u32 = boxes.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
        assert_eq!(cells, 8);
    }

    #[test]
    fn resizing_keeps_the_bottom_left() {
        let mut map = Tilemap::from_rows(&["#.", "##"], &[('#', 3)]);
        map.resize(3, 1);
        assert_eq!(map.cells, vec![3, 3, 0]);
        map.resize(1, 2);
        assert_eq!(map.cells, vec![3, 0]);
    }

    #[test]
    fn maps_save_as_rows_of_numbers_top_first() {
        let map = Tilemap::from_rows(&["-1 .", "##"], &[('#', 3), ('-', -1)]);
        let text = ron::to_string(&map).expect("saves");
        assert!(text.contains(r#"cells:["-1 0 0 0","3 3 0 0"]"#), "{text}");
        let back: Tilemap = ron::from_str(&text).expect("loads");
        assert_eq!(back, map);
        let flat: Tilemap = ron::from_str("(width: 2, height: 1, cells: [4, 0])").expect("flat");
        assert_eq!(flat.get(0, 0), 4);
    }

    #[test]
    fn frames_count_left_to_right_top_to_bottom() {
        assert_eq!(frame_uv(0, 4, 2), [0.0, 0.0, 0.25, 0.5]);
        assert_eq!(frame_uv(5, 4, 2), [0.25, 0.5, 0.5, 1.0]);
        assert_eq!(frame_uv(8, 4, 2), frame_uv(0, 4, 2));
    }
}
