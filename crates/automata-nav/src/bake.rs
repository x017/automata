//! Baking: triangles in, walkable rectangles and links out.
//!
//! 1. **Voxelise**: every triangle is clipped against the columns of a
//!    grid (`cell_size` wide) it covers; each piece adds a solid span
//!    (its lowest to highest point) to its column. Overlapping spans
//!    merge; a span is walkable when the triangle on top of it is flat
//!    enough.
//! 2. **Walkable cells**: the top of a walkable span with `agent_height`
//!    of headroom is a cell. Cells link to the cells beside them that are
//!    within `max_climb` and share the headroom.
//! 3. **Erode**: a distance transform measures every cell's distance to
//!    the nearest wall or ledge; cells nearer than `agent_radius` go, so
//!    an agent standing anywhere on the mesh clears the walls.
//! 4. **Regions**: connected cells are flood-filled into regions; tiny
//!    ones (`min_area`) are dropped.
//! 5. **Polygons**: each region is covered greedily with rectangles of
//!    cells that lie on one plane (stairs give one per step).
//! 6. **Links**: from every edge cell the bake looks straight out across
//!    the gap for a cell of another region, or one well below or above,
//!    within `jump_distance`; unobstructed ones become jump/drop links.

// Geometry reads best with short names (x, z, a, b, ...).
#![allow(clippy::many_single_char_names)]

use glam::Vec3;

use crate::{Link, NavMesh, NavSettings, Poly};

/// Why a bake produced nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum BakeError {
    /// There's no geometry to bake from (no static colliders).
    Empty,
    /// The level is too big for the cell size: `cells` columns.
    TooLarge {
        /// Columns the grid would have.
        cells: u64,
    },
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "nothing to bake: the scene has no static colliders"),
            Self::TooLarge { cells } => write!(
                f,
                "the level needs {cells} grid columns; make the cell size bigger"
            ),
        }
    }
}

impl std::error::Error for BakeError {}

/// The most grid columns a bake will use (2000 × 2000).
const MAX_CELLS: u64 = 4_000_000;
/// "No cell".
const NONE: u32 = u32::MAX;
/// The four neighbours: -x, +z, +x, -z.
const DX: [i64; 4] = [-1, 0, 1, 0];
const DZ: [i64; 4] = [0, 1, 0, -1];
/// Where jumps are looked for: the four sides, then the diagonals.
const JUMP_DIRS: [(i64, i64); 8] = [
    (-1, 0),
    (0, 1),
    (1, 0),
    (0, -1),
    (-1, 1),
    (1, 1),
    (1, -1),
    (-1, -1),
];
/// The two sides (indices into `DX`/`DZ`) each diagonal lies between.
const DIAG_SIDES: [[usize; 2]; 4] = [[0, 1], [2, 1], [2, 3], [0, 3]];
/// The longest side of a polygon, in cells (keeps paths from running far
/// along one huge rectangle's edge before turning).
const MAX_SIDE: usize = 48;
/// How far apart jump links of the same pair of regions must start.
const LINK_SPACING: f32 = 1.5;

#[derive(Debug, Clone, Copy)]
struct Span {
    min: f32,
    max: f32,
    walk: bool,
}

#[derive(Debug, Clone, Copy)]
struct Cell {
    x: u32,
    z: u32,
    y: f32,
    /// Where the solid above starts (infinity when open).
    top: f32,
    con: [u32; 4],
}

/// The voxel grid: columns of solid spans.
struct Grid {
    origin: [f32; 2],
    w: usize,
    h: usize,
    cs: f32,
    columns: Vec<Vec<Span>>,
}

impl Grid {
    fn column(&self, x: i64, z: i64) -> Option<usize> {
        let (w, h) = (i64::try_from(self.w).ok()?, i64::try_from(self.h).ok()?);
        (x >= 0 && z >= 0 && x < w && z < h)
            .then(|| usize::try_from(z * w + x).ok())
            .flatten()
    }
}

/// Bakes a navigation mesh from world-space triangles (see the module
/// docs for how), plus hand-placed `links` (kept as they are).
///
/// # Errors
///
/// [`BakeError::Empty`] without triangles, [`BakeError::TooLarge`] when
/// the grid would be unreasonably big.
pub fn bake(
    vertices: &[Vec3],
    triangles: &[[u32; 3]],
    links: &[Link],
    settings: NavSettings,
) -> Result<NavMesh, BakeError> {
    let settings = settings.sanitized();
    let hash = geometry_hash(vertices, triangles, links, &settings);
    let tris: Vec<[Vec3; 3]> = triangles
        .iter()
        .filter_map(|t| {
            let p = t.map(|i| vertices.get(i as usize).copied());
            let [Some(a), Some(b), Some(c)] = p else {
                return None;
            };
            (a.is_finite() && b.is_finite() && c.is_finite()).then_some([a, b, c])
        })
        .collect();
    if tris.is_empty() {
        return Err(BakeError::Empty);
    }
    let mut grid = voxelise(&tris, &settings)?;
    filter_low_obstacles(&mut grid, settings.max_climb);
    let mut cells = walkable_cells(&grid, &settings);
    connect(&grid, &mut cells, &settings);
    let mut alive = erode(&cells, &settings);
    let region = regions(&mut cells, &mut alive, &settings);
    let (polys, poly_of) = rectangles(&grid, &cells, &alive, &region, &settings);
    let mut all_links = if settings.jump_distance > 0.0 {
        jump_links(&grid, &cells, &poly_of, &region, &settings)
    } else {
        Vec::new()
    };
    all_links.extend(links.iter().map(|l| Link { auto: false, ..*l }));
    let size = [
        u32::try_from(grid.w).unwrap_or(u32::MAX),
        u32::try_from(grid.h).unwrap_or(u32::MAX),
    ];
    Ok(NavMesh::new(
        settings,
        grid.origin,
        size,
        polys,
        all_links,
        hash,
    ))
}

/// A fingerprint of a bake's inputs: the triangles (to the millimetre,
/// in any order), the hand-placed links and the settings. A saved mesh
/// whose hash differs from the level's was baked from something else.
#[must_use]
pub fn geometry_hash(
    vertices: &[Vec3],
    triangles: &[[u32; 3]],
    links: &[Link],
    settings: &NavSettings,
) -> u64 {
    fn fnv(h: &mut u64, v: u64) {
        for b in v.to_le_bytes() {
            *h ^= u64::from(b);
            *h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn mix(mut z: u64) -> u64 {
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // quantised
    let q = |v: f32| (v * 1000.0).round() as i64 as u64;
    let point = |h: &mut u64, p: Vec3| {
        fnv(h, q(p.x));
        fnv(h, q(p.y));
        fnv(h, q(p.z));
    };
    // Order-free: the sum of every triangle's own hash.
    let mut sum = 0u64;
    for t in triangles {
        let [Some(a), Some(b), Some(c)] = t.map(|i| vertices.get(i as usize).copied()) else {
            continue;
        };
        let mut h = 0xcbf2_9ce4_8422_2325;
        for p in [a, b, c] {
            point(&mut h, p);
        }
        sum = sum.wrapping_add(mix(h));
    }
    for l in links {
        let mut h = 0x1234_5678_9abc_def0;
        point(&mut h, l.start);
        point(&mut h, l.end);
        fnv(&mut h, u64::from(l.two_way));
        sum = sum.wrapping_add(mix(h));
    }
    let mut h = 0xcbf2_9ce4_8422_2325;
    fnv(&mut h, sum);
    for v in [
        settings.cell_size,
        settings.cell_height,
        settings.agent_radius,
        settings.agent_height,
        settings.max_climb,
        settings.max_slope,
        settings.jump_distance,
        settings.jump_height,
        settings.drop_height,
        settings.min_area,
    ] {
        fnv(&mut h, u64::from(v.to_bits()));
    }
    h
}

/// Clips a convex polygon to the side of an axis-aligned plane where
/// `coord(axis) * sign >= value * sign`.
fn clip(input: &[Vec3], out: &mut Vec<Vec3>, axis: usize, value: f32, sign: f32) {
    out.clear();
    let n = input.len();
    for i in 0..n {
        let p = input[i];
        let q = input[(i + 1) % n];
        let dp = (p[axis] - value) * sign;
        let dq = (q[axis] - value) * sign;
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dq >= 0.0) {
            let t = dp / (dp - dq);
            out.push(p + (q - p) * t);
        }
    }
}

#[allow(clippy::cast_possible_truncation)] // cell indices fit
fn cell_of(v: f32, origin: f32, cs: f32) -> i64 {
    ((v - origin) / cs).floor() as i64
}

fn voxelise(tris: &[[Vec3; 3]], s: &NavSettings) -> Result<Grid, BakeError> {
    let cs = s.cell_size;
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for t in tris {
        for p in t {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
    }
    // One empty column of margin all round, so every edge cell has an
    // outside neighbour.
    let origin = [lo.x - cs, lo.z - cs];
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // checked below
    let (w, h) = (
        ((hi.x - lo.x) / cs).ceil().max(0.0) as u64 + 3,
        ((hi.z - lo.z) / cs).ceil().max(0.0) as u64 + 3,
    );
    if w.saturating_mul(h) > MAX_CELLS {
        return Err(BakeError::TooLarge {
            cells: w.saturating_mul(h),
        });
    }
    let (w, h) = (
        usize::try_from(w).unwrap_or(usize::MAX),
        usize::try_from(h).unwrap_or(usize::MAX),
    );
    let mut grid = Grid {
        origin,
        w,
        h,
        cs,
        columns: vec![Vec::new(); w * h],
    };
    let cos_slope = s.max_slope.to_radians().cos();
    let (mut row, mut tmp, mut cell) = (Vec::new(), Vec::new(), Vec::new());
    for t in tris {
        let n = (t[1] - t[0]).cross(t[2] - t[0]);
        let len = n.length();
        if len <= 1e-12 {
            continue;
        }
        // Facing up (counter-clockwise seen from above): the underside
        // of a box is no floor, nor the hollow inside it.
        let walk = n.y / len >= cos_slope;
        let lo = t[0].min(t[1]).min(t[2]);
        let hi = t[0].max(t[1]).max(t[2]);
        let z0 = cell_of(lo.z, origin[1], cs).max(0);
        let z1 = cell_of(hi.z, origin[1], cs).min(i64::try_from(h).unwrap_or(i64::MAX) - 1);
        for z in z0..=z1 {
            let zlo = origin[1] + z as f32 * cs;
            clip(t, &mut tmp, 2, zlo, 1.0);
            clip(&tmp, &mut row, 2, zlo + cs, -1.0);
            if row.len() < 3 {
                continue;
            }
            let (rx0, rx1) = row
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
                    (a.min(p.x), b.max(p.x))
                });
            let x0 = cell_of(rx0, origin[0], cs).max(0);
            let x1 = cell_of(rx1, origin[0], cs).min(i64::try_from(w).unwrap_or(i64::MAX) - 1);
            for x in x0..=x1 {
                let xlo = origin[0] + x as f32 * cs;
                clip(&row, &mut tmp, 0, xlo, 1.0);
                clip(&tmp, &mut cell, 0, xlo + cs, -1.0);
                if cell.len() < 3 || on_column_edge(&cell, n / len, xlo, zlo, cs) {
                    continue;
                }
                let (min, max) = cell
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
                        (a.min(p.y), b.max(p.y))
                    });
                if let Some(c) = grid.column(x, z) {
                    add_span(&mut grid.columns[c], min, max, walk, s.cell_height);
                }
            }
        }
    }
    Ok(grid)
}

/// Whether a clipped piece only touches its column along one of the
/// column's sides and belongs to the column across: a face lying exactly
/// on a cell boundary counts for the side its solid is on (behind its
/// outward `normal`), and slivers of flat faces count for neither.
fn on_column_edge(piece: &[Vec3], normal: Vec3, xlo: f32, zlo: f32, cs: f32) -> bool {
    let eps = cs * 1e-3;
    let on = |axis: usize, v: f32| piece.iter().all(|p| (p[axis] - v).abs() <= eps);
    (on(0, xlo) && normal.x >= -1e-6)
        || (on(0, xlo + cs) && normal.x <= 1e-6)
        || (on(2, zlo) && normal.z >= -1e-6)
        || (on(2, zlo + cs) && normal.z <= 1e-6)
}

/// Adds a solid span to a column, merging it with the spans it overlaps.
/// The merged span is walkable if its top is: the top-most piece decides
/// (pieces within `merge` of each other on top both count).
fn add_span(column: &mut Vec<Span>, min: f32, max: f32, walk: bool, merge: f32) {
    let mut new = Span { min, max, walk };
    let mut i = 0;
    while i < column.len() {
        let s = column[i];
        if s.max < new.min || s.min > new.max {
            i += 1;
            continue;
        }
        if (s.max - new.max).abs() <= merge {
            new.walk |= s.walk;
        } else if s.max > new.max {
            new.walk = s.walk;
        }
        new.min = new.min.min(s.min);
        new.max = new.max.max(s.max);
        column.remove(i);
    }
    let at = column.partition_point(|s| s.min < new.min);
    column.insert(at, new);
}

/// Something low on a walkable floor (a kerb, a lip) is walkable too.
fn filter_low_obstacles(grid: &mut Grid, climb: f32) {
    for column in &mut grid.columns {
        let mut below_walk = false;
        let mut below_top = f32::NEG_INFINITY;
        for s in column.iter_mut() {
            let was = s.walk;
            if !s.walk && below_walk && s.max - below_top <= climb {
                s.walk = true;
            }
            below_walk = was;
            below_top = s.max;
        }
    }
}

fn walkable_cells(grid: &Grid, s: &NavSettings) -> Vec<Cell> {
    let mut cells = Vec::new();
    for z in 0..grid.h {
        for x in 0..grid.w {
            let column = &grid.columns[z * grid.w + x];
            for (i, span) in column.iter().enumerate() {
                if !span.walk {
                    continue;
                }
                let top = column.get(i + 1).map_or(f32::INFINITY, |n| n.min);
                if top - span.max >= s.agent_height {
                    cells.push(Cell {
                        x: u32::try_from(x).unwrap_or(u32::MAX),
                        z: u32::try_from(z).unwrap_or(u32::MAX),
                        y: span.max,
                        top,
                        con: [NONE; 4],
                    });
                }
            }
        }
    }
    cells
}

/// Index of each column's first cell (cells are sorted by column).
fn column_starts(grid: &Grid, cells: &[Cell]) -> Vec<u32> {
    let mut starts = vec![0u32; grid.w * grid.h + 1];
    for c in cells {
        starts[c.z as usize * grid.w + c.x as usize + 1] += 1;
    }
    for i in 1..starts.len() {
        starts[i] += starts[i - 1];
    }
    starts
}

fn connect(grid: &Grid, cells: &mut [Cell], s: &NavSettings) {
    let starts = column_starts(grid, cells);
    for i in 0..cells.len() {
        let c = cells[i];
        for d in 0..4 {
            let Some(col) = grid.column(i64::from(c.x) + DX[d], i64::from(c.z) + DZ[d]) else {
                continue;
            };
            let mut best = (NONE, f32::INFINITY);
            for j in starts[col]..starts[col + 1] {
                let n = cells[j as usize];
                let dy = (n.y - c.y).abs();
                let gap = c.top.min(n.top) - c.y.max(n.y);
                if dy <= s.max_climb && gap >= s.agent_height && dy < best.1 {
                    best = (j, dy);
                }
            }
            cells[i].con[d] = best.0;
        }
    }
}

/// The distance transform (chamfer, 2 per cell straight and 3 diagonal)
/// from the edges, then the cells nearer than the agent's radius go.
/// Returns which cells survive.
fn erode(cells: &[Cell], s: &NavSettings) -> Vec<bool> {
    let n = cells.len();
    let mut dist: Vec<u16> = cells
        .iter()
        .map(|c| if c.con.contains(&NONE) { 0 } else { u16::MAX })
        .collect();
    let step = |dist: &mut Vec<u16>, i: usize, j: u32, cost: u16| {
        if j != NONE {
            let v = dist[j as usize].saturating_add(cost);
            if v < dist[i] {
                dist[i] = v;
            }
        }
    };
    // Forward: from -x and -z (and their diagonals).
    for i in 0..n {
        let c = &cells[i];
        let a = c.con[0];
        step(&mut dist, i, a, 2);
        if a != NONE {
            step(&mut dist, i, cells[a as usize].con[3], 3);
        }
        let b = c.con[3];
        step(&mut dist, i, b, 2);
        if b != NONE {
            step(&mut dist, i, cells[b as usize].con[2], 3);
        }
    }
    // Backward: from +x and +z.
    for i in (0..n).rev() {
        let c = &cells[i];
        let a = c.con[2];
        step(&mut dist, i, a, 2);
        if a != NONE {
            step(&mut dist, i, cells[a as usize].con[1], 3);
        }
        let b = c.con[1];
        step(&mut dist, i, b, 2);
        if b != NONE {
            step(&mut dist, i, cells[b as usize].con[0], 3);
        }
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // small
    let keep_from = ((s.agent_radius / s.cell_size).ceil() * 2.0).min(60_000.0) as u16;
    dist.iter().map(|d| *d >= keep_from).collect()
}

/// Flood-fills the surviving cells into regions, drops the small ones and
/// cuts links to cells that are gone. Returns each cell's region.
fn regions(cells: &mut [Cell], alive: &mut [bool], s: &NavSettings) -> Vec<u32> {
    for cell in cells.iter_mut() {
        for j in &mut cell.con {
            if *j != NONE && !alive[*j as usize] {
                *j = NONE;
            }
        }
    }
    let mut region = vec![NONE; cells.len()];
    let mut sizes = Vec::new();
    let mut stack = Vec::new();
    for seed in 0..cells.len() {
        if !alive[seed] || region[seed] != NONE {
            continue;
        }
        let id = u32::try_from(sizes.len()).unwrap_or(NONE);
        let mut size = 0usize;
        region[seed] = id;
        stack.push(seed);
        while let Some(i) = stack.pop() {
            size += 1;
            for j in cells[i].con {
                if j != NONE && region[j as usize] == NONE {
                    region[j as usize] = id;
                    stack.push(j as usize);
                }
            }
        }
        sizes.push(size);
    }
    // Small regions go; the rest are numbered afresh.
    let min_cells = s.min_area / (s.cell_size * s.cell_size);
    let mut renumber = vec![NONE; sizes.len()];
    let mut next = 0;
    for (r, size) in sizes.iter().enumerate() {
        if *size as f32 >= min_cells {
            renumber[r] = next;
            next += 1;
        }
    }
    for i in 0..cells.len() {
        if region[i] != NONE {
            region[i] = renumber[region[i] as usize];
            if region[i] == NONE {
                alive[i] = false;
            }
        }
    }
    region
}

/// Covers the regions with flat rectangles of cells. Returns the polygons
/// and each cell's polygon.
fn rectangles(
    grid: &Grid,
    cells: &[Cell],
    alive: &[bool],
    region: &[u32],
    s: &NavSettings,
) -> (Vec<Poly>, Vec<u32>) {
    let cs = grid.cs;
    let tol = s.cell_height;
    // The steepest rise between neighbouring cells on one plane.
    let max_rise = s.max_slope.to_radians().tan() * cs + tol;
    let mut poly_of = vec![NONE; cells.len()];
    let mut polys = Vec::new();
    for seed in 0..cells.len() {
        if !alive[seed] || poly_of[seed] != NONE {
            continue;
        }
        let r = region[seed];
        let free = |i: u32, poly_of: &[u32]| {
            i != NONE && alive[i as usize] && poly_of[i as usize] == NONE && region[i as usize] == r
        };
        let y0 = cells[seed].y;
        // Along +x.
        let mut row = vec![u32::try_from(seed).unwrap_or(NONE)];
        let mut gx: Option<f32> = None;
        while row.len() < MAX_SIDE {
            let n = cells[*row.last().unwrap_or(&NONE) as usize].con[2];
            if !free(n, &poly_of) {
                break;
            }
            let k = row.len() as f32;
            let y = cells[n as usize].y;
            match gx {
                None => {
                    let g = (y - y0) / k;
                    if g.abs() > max_rise {
                        break;
                    }
                    gx = Some(g);
                }
                Some(g) => {
                    if (y - (y0 + g * k)).abs() > tol {
                        break;
                    }
                }
            }
            row.push(n);
        }
        let gx = gx.unwrap_or(0.0);
        // Then whole rows along +z.
        let mut rows = vec![row];
        let mut gz: Option<f32> = None;
        'grow: while rows.len() < MAX_SIDE {
            let prev = &rows[rows.len() - 1];
            let mut next: Vec<u32> = Vec::with_capacity(prev.len());
            for (k, &p) in prev.iter().enumerate() {
                let n = cells[p as usize].con[1];
                if !free(n, &poly_of) || (k > 0 && cells[next[k - 1] as usize].con[2] != n) {
                    break 'grow;
                }
                next.push(n);
            }
            let rz = rows.len() as f32;
            let g = gz.unwrap_or((cells[next[0] as usize].y - y0) / rz);
            if g.abs() > max_rise {
                break;
            }
            for (k, &n) in next.iter().enumerate() {
                let want = y0 + gx * k as f32 + g * rz;
                if (cells[n as usize].y - want).abs() > tol {
                    break 'grow;
                }
            }
            gz = Some(g);
            rows.push(next);
        }
        let gz = gz.unwrap_or(0.0);
        let id = u32::try_from(polys.len()).unwrap_or(NONE);
        for row in &rows {
            for &c in row {
                poly_of[c as usize] = id;
            }
        }
        let c = cells[seed];
        let width = u32::try_from(rows[0].len()).unwrap_or(1);
        let depth = u32::try_from(rows.len()).unwrap_or(1);
        polys.push(Poly {
            min: [c.x, c.z],
            max: [c.x + width, c.z + depth],
            plane: [y0, gx / cs, gz / cs],
            region: r,
        });
    }
    (polys, poly_of)
}

/// Jumps and drops: from each edge cell, straight out across the gap.
fn jump_links(
    grid: &Grid,
    cells: &[Cell],
    poly_of: &[u32],
    region: &[u32],
    s: &NavSettings,
) -> Vec<Link> {
    let starts = column_starts(grid, cells);
    let cs = grid.cs;
    #[allow(clippy::cast_possible_truncation)] // small
    let max_steps = (s.jump_distance / cs).floor() as i64;
    let center = |c: &Cell| {
        Vec3::new(
            grid.origin[0] + (c.x as f32 + 0.5) * cs,
            c.y,
            grid.origin[1] + (c.z as f32 + 0.5) * cs,
        )
    };
    let mut links: Vec<(Link, u32, u32)> = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        if poly_of[i] == NONE {
            continue;
        }
        for (d, &(dx, dz)) in JUMP_DIRS.iter().enumerate() {
            // Straight out over an open edge, or diagonally off a corner
            // (gaps between turned platforms).
            let open = if d < 4 {
                c.con[d] == NONE
            } else {
                c.con[DIAG_SIDES[d - 4][0]] == NONE || c.con[DIAG_SIDES[d - 4][1]] == NONE
            };
            if !open {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)] // small
            let reach = if d < 4 {
                max_steps
            } else {
                (max_steps as f32 * std::f32::consts::FRAC_1_SQRT_2).floor() as i64
            };
            let Some((j, steps)) =
                probe(grid, cells, &starts, poly_of, region, s, i, (dx, dz), reach)
            else {
                continue;
            };
            let t = &cells[j];
            let (start, end) = (center(c), center(t));
            let (ra, rb) = (region[i], region[j]);
            let near = |l: &Link, a: Vec3| l.start.distance(a) < LINK_SPACING;
            let duplicate = links.iter().any(|(l, la, lb)| {
                (*la == ra && *lb == rb && near(l, start))
                    || (l.two_way && *la == rb && *lb == ra && l.end.distance(start) < LINK_SPACING)
            });
            if duplicate || steps == 0 {
                continue;
            }
            let dy = end.y - start.y;
            links.push((
                Link {
                    start,
                    end,
                    two_way: dy >= -s.jump_height && dy <= s.drop_height,
                    auto: true,
                },
                ra,
                rb,
            ));
        }
    }
    links.into_iter().map(|(l, _, _)| l).collect()
}

/// Looks out from cell `i` in direction `d` for where a jump lands: the
/// first column with a cell at a reachable height. Returns that cell and
/// how many columns out it is, or `None` when the way is blocked, the
/// cell beyond is the same floor, or nothing is in reach.
#[allow(clippy::too_many_arguments)]
fn probe(
    grid: &Grid,
    cells: &[Cell],
    starts: &[u32],
    poly_of: &[u32],
    region: &[u32],
    s: &NavSettings,
    i: usize,
    (dx, dz): (i64, i64),
    max_steps: i64,
) -> Option<(usize, i64)> {
    let c = &cells[i];
    for step in 1..=max_steps {
        let col = grid.column(i64::from(c.x) + dx * step, i64::from(c.z) + dz * step)?;
        let mut found = None;
        for j in starts[col]..starts[col + 1] {
            let j = j as usize;
            if poly_of[j] == NONE {
                continue;
            }
            let dy = cells[j].y - c.y;
            if dy < -s.drop_height || dy > s.jump_height {
                continue;
            }
            if found.is_none_or(|f: usize| (cells[f].y - c.y).abs() > dy.abs()) {
                found = Some(j);
            }
        }
        if let Some(j) = found {
            let dy = cells[j].y - c.y;
            // The same floor round a corner: walking is the way.
            if region[j] == region[i] && dy.abs() <= s.max_climb {
                return None;
            }
            // Nothing solid may stand in the way: a jump goes up first and
            // a drop goes out first, so the way across is at the higher end.
            let y = c.y.max(cells[j].y);
            for back in 1..step {
                let col = grid.column(i64::from(c.x) + dx * back, i64::from(c.z) + dz * back)?;
                if grid.columns[col]
                    .iter()
                    .any(|sp| sp.max > y + s.max_climb && sp.min < y + s.agent_height * 0.5)
                {
                    return None;
                }
            }
            return Some((j, step));
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An axis-aligned box's 12 triangles (outward winding).
    pub(crate) fn add_box(v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, lo: Vec3, hi: Vec3) {
        let b = u32::try_from(v.len()).unwrap();
        for i in 0..8 {
            v.push(Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            ));
        }
        for f in [
            [0, 4, 6, 2],
            [1, 3, 7, 5],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 2, 3, 1],
            [4, 5, 7, 6],
        ] {
            t.push([b + f[0], b + f[1], b + f[2]]);
            t.push([b + f[0], b + f[2], b + f[3]]);
        }
    }

    #[test]
    fn a_floor_bakes_into_one_eroded_rectangle() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-5.0, -1.0, -5.0),
            Vec3::new(5.0, 0.0, 5.0),
        );
        let mesh = bake(&v, &t, &[], NavSettings::default()).unwrap();
        assert_eq!(mesh.regions, 1);
        // 10 × 10 m, less 0.4 m all round (the radius, rounded to cells).
        assert!((mesh.area() - 9.2 * 9.2).abs() < 0.5, "{}", mesh.area());
        for i in 0..mesh.polys.len() {
            for c in mesh.poly_corners(i) {
                assert!(
                    c.y.abs() < 1e-4 && c.x.abs() <= 4.61 && c.z.abs() <= 4.61,
                    "{c}"
                );
            }
        }
        assert_eq!(mesh.links.len(), 0);
    }

    #[test]
    fn a_wall_is_cut_out_with_the_agents_radius() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-5.0, -1.0, -5.0),
            Vec3::new(5.0, 0.0, 5.0),
        );
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-0.3, 0.0, -3.0),
            Vec3::new(0.3, 2.0, 3.0),
        );
        let mesh = bake(&v, &t, &[], NavSettings::default()).unwrap();
        // The wall's top is too narrow to stand on once eroded; the floor
        // round it is one region.
        assert_eq!(mesh.regions, 1);
        for i in 0..mesh.polys.len() {
            let [a, _, b, _] = mesh.poly_corners(i);
            let (lo, hi) = (a.min(b), a.max(b));
            let dx = (-0.3 - hi.x).max(lo.x - 0.3).max(0.0);
            let dz = (-3.0 - hi.z).max(lo.z - 3.0).max(0.0);
            let gap = dx.hypot(dz);
            assert!(gap >= 0.34, "polygon {lo}..{hi} is {gap} from the wall");
        }
    }

    #[test]
    fn ramps_and_steps_are_walkable_and_cliffs_are_not() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-6.0, -1.0, -2.0),
            Vec3::new(6.0, 0.0, 2.0),
        );
        // Steps of 0.3 m up to a landing at 1.2 m.
        for k in 0..4 {
            let x = 6.0 + k as f32 * 0.5;
            add_box(
                &mut v,
                &mut t,
                Vec3::new(x, -1.0, -2.0),
                Vec3::new(x + 0.5, 0.3 * (k + 1) as f32, 2.0),
            );
        }
        add_box(
            &mut v,
            &mut t,
            Vec3::new(8.0, -1.0, -2.0),
            Vec3::new(12.0, 1.2, 2.0),
        );
        // A 2 m cliff to a ledge beyond.
        add_box(
            &mut v,
            &mut t,
            Vec3::new(12.0, -1.0, -2.0),
            Vec3::new(16.0, 3.2, 2.0),
        );
        let s = NavSettings {
            jump_distance: 0.0,
            ..NavSettings::default()
        };
        let mesh = bake(&v, &t, &[], s).unwrap();
        assert_eq!(
            mesh.regions, 2,
            "the floor and steps are one, the ledge another"
        );
    }

    #[test]
    fn gaps_get_jump_links_and_drops_go_one_way() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-4.0, -1.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        // 1 m across, level: a two-way jump.
        add_box(
            &mut v,
            &mut t,
            Vec3::new(1.0, -1.0, -2.0),
            Vec3::new(5.0, 0.0, 2.0),
        );
        // 0.8 m across and 2.5 m down: a drop, one way.
        add_box(
            &mut v,
            &mut t,
            Vec3::new(5.8, -3.5, -2.0),
            Vec3::new(9.8, -2.5, 2.0),
        );
        let mesh = bake(&v, &t, &[], NavSettings::default()).unwrap();
        assert_eq!(mesh.regions, 3);
        let jumps: Vec<_> = mesh.links.iter().filter(|l| l.end.y > -1.0).collect();
        let drops: Vec<_> = mesh.links.iter().filter(|l| l.end.y < -1.0).collect();
        assert!(!jumps.is_empty() && jumps.iter().all(|l| l.two_way && l.auto));
        assert!(!drops.is_empty() && drops.iter().all(|l| !l.two_way));
        // Spaced out along the edge, not one per cell.
        assert!(jumps.len() <= 4, "{}", jumps.len());
        let s = NavSettings {
            jump_distance: 0.0,
            ..NavSettings::default()
        };
        assert_eq!(bake(&v, &t, &[], s).unwrap().links.len(), 0);
    }

    #[test]
    fn nothing_to_bake_and_huge_levels_say_so() {
        assert_eq!(
            bake(&[], &[], &[], NavSettings::default()).unwrap_err(),
            BakeError::Empty
        );
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(&mut v, &mut t, Vec3::splat(-1.0e4), Vec3::splat(1.0e4));
        assert!(matches!(
            bake(&v, &t, &[], NavSettings::default()),
            Err(BakeError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_hash_ignores_triangle_order_but_not_geometry() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(&mut v, &mut t, Vec3::ZERO, Vec3::ONE);
        let s = NavSettings::default();
        let h = geometry_hash(&v, &t, &[], &s);
        let mut r = t.clone();
        r.reverse();
        assert_eq!(geometry_hash(&v, &r, &[], &s), h);
        v[0].x += 0.01;
        assert_ne!(geometry_hash(&v, &t, &[], &s), h);
        let mut s2 = s;
        s2.agent_radius = 0.5;
        assert_ne!(
            geometry_hash(&v, &t, &[], &s2),
            geometry_hash(&v, &t, &[], &s)
        );
    }
}
