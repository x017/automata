//! Queries on a baked mesh: where a point is, the way between two.

// Geometry reads best with short names (x, z, a, b, ...).
#![allow(clippy::many_single_char_names)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::{Vec2, Vec3};

use crate::NavMesh;

/// "None".
const NONE: u32 = u32::MAX;
/// Extra cost of taking a link, on top of its length (seconds' worth of
/// metres: a jump is slower than walking the same distance).
const LINK_PENALTY: f32 = 1.0;
/// How many polygons a search may expand before giving up.
const MAX_EXPANDED: usize = 65_536;
/// How far below a point the floor under it is looked for (metres).
const LOOK_BELOW: f32 = 4.0;
/// How far above it.
const LOOK_ABOVE: f32 = 1.0;
/// How far off the mesh a point may be and still snap onto it (metres).
const SNAP: f32 = 2.0;

/// One corner of a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathPoint {
    /// Where (on the mesh).
    pub position: Vec3,
    /// Whether the way here from the previous corner is an off-mesh link
    /// (a jump or a drop) rather than a walk.
    pub link: bool,
}

/// A way out of a polygon: a shared edge or a link.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Edge {
    pub to: u32,
    /// The shared edge's ends, or the link's start and end.
    pub a: Vec3,
    pub b: Vec3,
    /// The link's index, or `NONE` for an edge.
    pub link: u32,
}

/// The mesh's connectivity, rebuilt on load.
#[derive(Debug, Clone, Default)]
pub(crate) struct Graph {
    /// Per grid column, the range of `col_polys` covering it.
    col_start: Vec<u32>,
    col_polys: Vec<u32>,
    pub edges: Vec<Vec<Edge>>,
}

impl Graph {
    pub fn build(mesh: &NavMesh) -> Self {
        let [w, h] = mesh.size.map(|v| v as usize);
        let mut g = Self {
            col_start: vec![0; w * h + 1],
            col_polys: Vec::new(),
            edges: vec![Vec::new(); mesh.polys.len()],
        };
        if w == 0 || h == 0 {
            return g;
        }
        for p in &mesh.polys {
            for z in p.min[1]..p.max[1] {
                for x in p.min[0]..p.max[0] {
                    g.col_start[z as usize * w + x as usize + 1] += 1;
                }
            }
        }
        for i in 1..g.col_start.len() {
            g.col_start[i] += g.col_start[i - 1];
        }
        let mut fill = g.col_start.clone();
        g.col_polys = vec![0; g.col_start[w * h] as usize];
        for (i, p) in mesh.polys.iter().enumerate() {
            for z in p.min[1]..p.max[1] {
                for x in p.min[0]..p.max[0] {
                    let c = z as usize * w + x as usize;
                    g.col_polys[fill[c] as usize] = u32::try_from(i).unwrap_or(NONE);
                    fill[c] += 1;
                }
            }
        }
        g.connect(mesh);
        g.add_links(mesh);
        g
    }

    pub fn column(&self, mesh: &NavMesh, x: i64, z: i64) -> &[u32] {
        let [w, h] = mesh.size.map(i64::from);
        if x < 0 || z < 0 || x >= w || z >= h || self.col_start.is_empty() {
            return &[];
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // checked
        let c = (z * w + x) as usize;
        &self.col_polys[self.col_start[c] as usize..self.col_start[c + 1] as usize]
    }

    /// Joins polygons that share an edge at heights an agent can step.
    fn connect(&mut self, mesh: &NavMesh) {
        let cs = mesh.settings.cell_size;
        let climb = mesh.settings.max_climb + mesh.settings.cell_height;
        for i in 0..mesh.polys.len() {
            let p = mesh.polys[i];
            // The +x and +z sides; the other two come from the neighbours.
            for axis in 0..2 {
                let other = 1 - axis;
                let edge = p.max[axis];
                if edge >= mesh.size[axis] {
                    continue;
                }
                let mut seen: Vec<u32> = Vec::new();
                for k in p.min[other]..p.max[other] {
                    let (x, z) = if axis == 0 { (edge, k) } else { (k, edge) };
                    let column = self.column(mesh, i64::from(x), i64::from(z)).to_vec();
                    for j in column {
                        let q = mesh.polys[j as usize];
                        if q.min[axis] != edge || seen.contains(&j) {
                            continue;
                        }
                        seen.push(j);
                        let lo = p.min[other].max(q.min[other]);
                        let hi = p.max[other].min(q.max[other]);
                        if lo >= hi {
                            continue;
                        }
                        let e = mesh.origin[axis] + edge as f32 * cs;
                        let a = mesh.origin[other] + lo as f32 * cs;
                        let b = mesh.origin[other] + hi as f32 * cs;
                        let point = |t: f32| {
                            if axis == 0 {
                                (e, t)
                            } else {
                                (t, e)
                            }
                        };
                        // Heights on both sides, half a cell in from the ends.
                        let mut ends = [Vec3::ZERO; 2];
                        let mut ok = true;
                        for (n, t) in [a + cs * 0.5, b - cs * 0.5].into_iter().enumerate() {
                            let (x, z) = point(t);
                            let hp = mesh.plane_height(i, x, z);
                            let hq = mesh.plane_height(j as usize, x, z);
                            ok &= (hp - hq).abs() <= climb;
                            let (x, z) = point(if n == 0 { a } else { b });
                            ends[n] = Vec3::new(x, f32::midpoint(hp, hq), z);
                        }
                        if !ok {
                            continue;
                        }
                        self.edges[i].push(Edge {
                            to: j,
                            a: ends[0],
                            b: ends[1],
                            link: NONE,
                        });
                        self.edges[j as usize].push(Edge {
                            to: u32::try_from(i).unwrap_or(NONE),
                            a: ends[0],
                            b: ends[1],
                            link: NONE,
                        });
                    }
                }
            }
        }
    }

    fn add_links(&mut self, mesh: &NavMesh) {
        for (k, l) in mesh.links.iter().enumerate() {
            let link = u32::try_from(k).unwrap_or(NONE);
            let (Some((s, a)), Some((e, b))) = (
                mesh.locate_with(self, l.start, 1.0, 1.0, 1.0),
                mesh.locate_with(self, l.end, 1.0, 1.0, 1.0),
            ) else {
                continue;
            };
            if s == e {
                continue;
            }
            self.edges[s as usize].push(Edge { to: e, a, b, link });
            if l.two_way {
                self.edges[e as usize].push(Edge {
                    to: s,
                    a: b,
                    b: a,
                    link,
                });
            }
        }
    }
}

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// Twice the signed area of triangle `a b c` seen from above (the funnel
/// algorithm's turn test).
fn triarea2(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let (ax, ay) = (b.x - a.x, b.y - a.y);
    let (bx, by) = (c.x - a.x, c.y - a.y);
    bx * ay - ax * by
}

/// The closest point to `p` on segment `a b`.
fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 1e-12 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

/// Pulls a path through a run of portals tight (the "simple stupid
/// funnel algorithm"). `portals` are `(left, right)` pairs; the first is
/// the start, the last the end (both ends the same point). Corners are
/// appended to `out`, the start included.
fn string_pull(portals: &[(Vec3, Vec3)], out: &mut Vec<Vec3>) {
    let Some(&(start, _)) = portals.first() else {
        return;
    };
    let same = |a: Vec3, b: Vec3| flat(a).distance_squared(flat(b)) < 1e-8;
    let (mut apex, mut left, mut right) = (start, start, start);
    let (mut left_i, mut right_i) = (0, 0);
    out.push(start);
    let mut i = 1;
    while i < portals.len() {
        let (l, r) = portals[i];
        let (fa, fl, fr) = (flat(apex), flat(left), flat(right));
        // Narrow the right side.
        if triarea2(fa, fr, flat(r)) <= 0.0 {
            if same(apex, right) || triarea2(fa, fl, flat(r)) > 0.0 {
                right = r;
                right_i = i;
            } else {
                // The right crossed the left: the left is a corner.
                apex = left;
                let apex_i = left_i;
                out.push(apex);
                (left, right) = (apex, apex);
                (left_i, right_i) = (apex_i, apex_i);
                i = apex_i + 1;
                continue;
            }
        }
        let (fa, fl, fr) = (flat(apex), flat(left), flat(right));
        // Narrow the left side.
        if triarea2(fa, fl, flat(l)) >= 0.0 {
            if same(apex, left) || triarea2(fa, fr, flat(l)) < 0.0 {
                left = l;
                left_i = i;
            } else {
                apex = right;
                let apex_i = right_i;
                out.push(apex);
                (left, right) = (apex, apex);
                (left_i, right_i) = (apex_i, apex_i);
                i = apex_i + 1;
                continue;
            }
        }
        i += 1;
    }
    if let Some(&(end, _)) = portals.last() {
        if out
            .last()
            .is_none_or(|p| !same(*p, end) || (p.y - end.y).abs() > 1e-3)
        {
            out.push(end);
        }
    }
}

impl NavMesh {
    /// The polygon under `p` and the point on it straight below (or just
    /// above): searches from `below` metres under `p` to `above` over it,
    /// then sideways up to `snap` metres. `None` when there's no mesh
    /// there.
    #[must_use]
    pub fn locate(&self, p: Vec3, below: f32, above: f32, snap: f32) -> Option<(u32, Vec3)> {
        self.locate_with(&self.graph, p, below, above, snap)
    }

    fn locate_with(
        &self,
        graph: &Graph,
        p: Vec3,
        below: f32,
        above: f32,
        snap: f32,
    ) -> Option<(u32, Vec3)> {
        if !p.is_finite() || self.polys.is_empty() {
            return None;
        }
        let cs = self.settings.cell_size;
        #[allow(clippy::cast_possible_truncation)] // grid indices
        let (cx, cz) = (
            ((p.x - self.origin[0]) / cs).floor() as i64,
            ((p.z - self.origin[1]) / cs).floor() as i64,
        );
        let mut best: Option<(u32, f32, f32)> = None;
        for &j in graph.column(self, cx, cz) {
            let h = self.plane_height(j as usize, p.x, p.z);
            if h > p.y + above || h < p.y - below {
                continue;
            }
            // Prefer the floor under the point to a ceiling over it.
            let score = if h <= p.y { p.y - h } else { (h - p.y) * 3.0 };
            if best.is_none_or(|b| score < b.1) {
                best = Some((j, score, h));
            }
        }
        if let Some((j, _, h)) = best {
            return Some((j, Vec3::new(p.x, h, p.z)));
        }
        if snap <= 0.0 {
            return None;
        }
        // Off the mesh: the nearest point of any polygon in reach.
        let mut near: Option<(u32, f32, Vec3)> = None;
        for (j, poly) in self.polys.iter().enumerate() {
            let x0 = self.origin[0] + poly.min[0] as f32 * cs;
            let z0 = self.origin[1] + poly.min[1] as f32 * cs;
            let x1 = self.origin[0] + poly.max[0] as f32 * cs;
            let z1 = self.origin[1] + poly.max[1] as f32 * cs;
            let inset = (cs * 0.01).min((x1 - x0) * 0.25);
            let x = p.x.clamp(x0 + inset, x1 - inset);
            let z = p.z.clamp(z0 + inset, z1 - inset);
            let d = Vec2::new(x - p.x, z - p.z).length();
            if d > snap {
                continue;
            }
            let h = self.plane_height(j, x, z);
            if h > p.y + above || h < p.y - below {
                continue;
            }
            let dist = d + (p.y - h).abs() * 0.25;
            if near.is_none_or(|n| dist < n.1) {
                near = Some((u32::try_from(j).unwrap_or(NONE), dist, Vec3::new(x, h, z)));
            }
        }
        near.map(|(j, _, q)| (j, q))
    }

    /// The point on the mesh nearest `p` within `max_distance` metres
    /// (straight down first), or `None`.
    #[must_use]
    pub fn nearest(&self, p: Vec3, max_distance: f32) -> Option<Vec3> {
        let r = max_distance.max(0.0);
        self.locate(p, LOOK_BELOW.max(r), LOOK_ABOVE.max(r), r)
            .map(|(_, q)| q)
    }

    /// The height of the mesh under `p` (looking a few metres down), or
    /// `None` off the mesh.
    #[must_use]
    pub fn height_at(&self, p: Vec3) -> Option<f32> {
        self.locate(p, LOOK_BELOW, LOOK_ABOVE, 0.0)
            .map(|(_, q)| q.y)
    }

    /// The way from `from` to `to` as corners, both ends included (snapped
    /// onto the mesh). Points a little off the mesh snap onto it. With
    /// `partial`, an unreachable `to` gives the way to the reachable point
    /// nearest it instead of `None`.
    #[must_use]
    pub fn find_path(&self, from: Vec3, to: Vec3, partial: bool) -> Option<Vec<PathPoint>> {
        let (start, a) = self.locate(from, LOOK_BELOW, LOOK_ABOVE, SNAP)?;
        let goal = self.locate(to, LOOK_BELOW, LOOK_ABOVE, SNAP);
        if goal.is_none() && !partial {
            return None;
        }
        let target = goal.map_or(to, |(_, b)| b);
        let goal_poly = goal.map_or(NONE, |(g, _)| g);
        let (reached, end_poly, scratch) = self.search(start, a, goal_poly, target);
        if !reached && !partial {
            return None;
        }
        let end = if reached {
            target
        } else {
            self.closest_in_poly(end_poly, target)
        };
        let chain = self.chain(&scratch, end_poly);
        Some(self.pull(&chain, a, end))
    }

    /// A* from polygon `start` (entered at `a`) towards `goal`. Returns
    /// whether it got there, the polygon it ended in (the goal, or the
    /// one nearest `target`) and the search's state.
    fn search(&self, start: u32, a: Vec3, goal: u32, target: Vec3) -> (bool, u32, Scratch) {
        let mut s = Scratch::new(self.polys.len());
        s.g[start as usize] = 0.0;
        s.entry[start as usize] = a;
        let mut heap = BinaryHeap::new();
        heap.push(Reverse((a.distance(target).to_bits(), start)));
        let mut best = (start, a.distance(target));
        let mut expanded = 0;
        while let Some(Reverse((_, i))) = heap.pop() {
            let iu = i as usize;
            if s.closed[iu] {
                continue;
            }
            s.closed[iu] = true;
            if i == goal {
                return (true, i, s);
            }
            let here = s.entry[iu];
            let h = here.distance(target);
            if h < best.1 {
                best = (i, h);
            }
            expanded += 1;
            if expanded > MAX_EXPANDED {
                break;
            }
            for (k, e) in self.graph.edges[iu].iter().enumerate() {
                let j = e.to as usize;
                if s.closed[j] {
                    continue;
                }
                let (entry, cost) = if e.link == NONE {
                    let p = closest_on_segment(here, e.a, e.b);
                    (p, here.distance(p))
                } else {
                    (e.b, here.distance(e.a) + e.a.distance(e.b) + LINK_PENALTY)
                };
                let g = s.g[iu] + cost;
                if g < s.g[j] {
                    s.g[j] = g;
                    s.entry[j] = entry;
                    s.parent[j] = (i, u32::try_from(k).unwrap_or(NONE));
                    heap.push(Reverse(((g + entry.distance(target)).to_bits(), e.to)));
                }
            }
        }
        (false, best.0, s)
    }

    /// The polygons from the search's start to `end`, each with the edge
    /// it was entered by (an index into the previous one's edges).
    fn chain(&self, s: &Scratch, end: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut at = end;
        loop {
            let (parent, edge) = s.parent[at as usize];
            out.push((at, edge));
            if parent == NONE || out.len() > self.polys.len() {
                break;
            }
            at = parent;
        }
        out.reverse();
        out
    }

    /// Corners along a chain of polygons, links included.
    fn pull(&self, chain: &[(u32, u32)], start: Vec3, end: Vec3) -> Vec<PathPoint> {
        let mut out: Vec<PathPoint> = Vec::new();
        let mut portals: Vec<(Vec3, Vec3)> = vec![(start, start)];
        let mut corners = Vec::new();
        let mut flush = |portals: &mut Vec<(Vec3, Vec3)>, to: Vec3, out: &mut Vec<PathPoint>| {
            portals.push((to, to));
            corners.clear();
            string_pull(portals, &mut corners);
            for (n, c) in corners.iter().enumerate() {
                // The run's start is already there (the previous run's end).
                if n == 0 && !out.is_empty() {
                    continue;
                }
                out.push(PathPoint {
                    position: *c,
                    link: false,
                });
            }
            portals.clear();
        };
        for w in chain.windows(2) {
            let (from, _) = w[0];
            let (to, edge) = w[1];
            let e = self.graph.edges[from as usize][edge as usize];
            if e.link == NONE {
                // Left and right as seen going from `from` into `to`.
                let dir = flat(self.poly_center(to as usize) - self.poly_center(from as usize));
                let side = |p: Vec3| {
                    let m = flat((e.a + e.b) * 0.5);
                    let d = flat(p) - m;
                    dir.x * d.y - dir.y * d.x
                };
                let (l, r) = if side(e.a) > 0.0 {
                    (e.a, e.b)
                } else {
                    (e.b, e.a)
                };
                portals.push((l, r));
            } else {
                flush(&mut portals, e.a, &mut out);
                out.push(PathPoint {
                    position: e.b,
                    link: true,
                });
                portals.push((e.b, e.b));
            }
        }
        flush(&mut portals, end, &mut out);
        out
    }

    /// The centre of polygon `i`'s rectangle, on its plane.
    #[must_use]
    pub fn poly_center(&self, i: usize) -> Vec3 {
        let p = &self.polys[i];
        let cs = self.settings.cell_size;
        let x = self.origin[0] + (p.min[0] + p.max[0]) as f32 * 0.5 * cs;
        let z = self.origin[1] + (p.min[1] + p.max[1]) as f32 * 0.5 * cs;
        Vec3::new(x, self.plane_height(i, x, z), z)
    }

    fn closest_in_poly(&self, i: u32, p: Vec3) -> Vec3 {
        let poly = &self.polys[i as usize];
        let cs = self.settings.cell_size;
        let x0 = self.origin[0] + poly.min[0] as f32 * cs;
        let z0 = self.origin[1] + poly.min[1] as f32 * cs;
        let x1 = self.origin[0] + poly.max[0] as f32 * cs;
        let z1 = self.origin[1] + poly.max[1] as f32 * cs;
        let x = p.x.clamp(x0, x1);
        let z = p.z.clamp(z0, z1);
        Vec3::new(x, self.plane_height(i as usize, x, z), z)
    }

    /// Moves from `from` towards `to` without leaving the mesh: slides
    /// along an edge it would cross, or stays put. Returns where it ends
    /// up, on the mesh. Something already off the mesh (placed beside it,
    /// or knocked off) moves freely, level with the mesh close by, until
    /// it's back on.
    #[must_use]
    pub fn move_along(&self, from: Vec3, to: Vec3) -> Vec3 {
        let climb = self.settings.max_climb + self.settings.cell_height;
        let try_at = |p: Vec3| self.locate(p, climb + 0.5, climb, 0.0).map(|(_, q)| q);
        if let Some(q) = try_at(to) {
            return q;
        }
        if try_at(from).is_none() {
            // Level with the mesh nearby, so it lines up to step back on.
            return self
                .nearest(to, 1.0)
                .map_or(to, |q| Vec3::new(to.x, q.y, to.z));
        }
        for p in [Vec3::new(to.x, to.y, from.z), Vec3::new(from.x, to.y, to.z)] {
            if let Some(q) = try_at(p) {
                return q;
            }
        }
        try_at(from).unwrap_or(from)
    }

    /// The mesh as plain numbers, for path finding elsewhere (Oberon's
    /// script bridge): per polygon `[x0, z0, x1, z1, y0, dx, dz]` (its
    /// rectangle, and its plane: height `y0` at `(x0, z0)` rising `dx`
    /// per metre along x and `dz` along z), and per polygon its ways out
    /// as `(to, a, b, link)` (a shared edge's ends, or a link's start and
    /// end).
    #[must_use]
    #[allow(clippy::type_complexity)] // plain data by design
    pub fn export(&self) -> (Vec<[f32; 7]>, Vec<Vec<(u32, Vec3, Vec3, bool)>>) {
        let polys = (0..self.polys.len())
            .map(|i| {
                let [c0, _, c2, _] = self.poly_corners(i);
                let p = &self.polys[i];
                [c0.x, c0.z, c2.x, c2.z, c0.y, p.plane[1], p.plane[2]]
            })
            .collect();
        let edges = self
            .graph
            .edges
            .iter()
            .map(|es| {
                es.iter()
                    .map(|e| (e.to, e.a, e.b, e.link != NONE))
                    .collect()
            })
            .collect();
        (polys, edges)
    }

    /// How many polygons and links lead out of polygon `i` (for tests and
    /// the overlay).
    #[must_use]
    pub fn neighbours(&self, i: usize) -> usize {
        self.graph.edges.get(i).map_or(0, Vec::len)
    }
}

/// A search's per-polygon state.
#[derive(Debug, Clone)]
struct Scratch {
    g: Vec<f32>,
    entry: Vec<Vec3>,
    parent: Vec<(u32, u32)>,
    closed: Vec<bool>,
}

impl Scratch {
    fn new(n: usize) -> Self {
        Self {
            g: vec![f32::INFINITY; n],
            entry: vec![Vec3::ZERO; n],
            parent: vec![(NONE, NONE); n],
            closed: vec![false; n],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bake, Link, NavSettings};

    fn add_box(v: &mut Vec<Vec3>, t: &mut Vec<[u32; 3]>, lo: Vec3, hi: Vec3) {
        crate::bake::tests::add_box(v, t, lo, hi);
    }

    fn floor_with_wall() -> NavMesh {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-6.0, -1.0, -6.0),
            Vec3::new(6.0, 0.0, 6.0),
        );
        // A wall across the middle with a way round at +z only.
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-0.5, 0.0, -6.0),
            Vec3::new(0.5, 2.0, 3.0),
        );
        bake(&v, &t, &[], NavSettings::default()).unwrap()
    }

    fn length(path: &[PathPoint]) -> f32 {
        path.windows(2)
            .map(|w| w[0].position.distance(w[1].position))
            .sum()
    }

    #[test]
    fn the_export_lists_every_polygon_and_its_ways_out() {
        let mesh = floor_with_wall();
        let (polys, edges) = mesh.export();
        assert_eq!(polys.len(), mesh.polys.len());
        assert_eq!(edges.len(), polys.len());
        for (i, p) in polys.iter().enumerate() {
            assert!(p[0] < p[2] && p[1] < p[3], "a rectangle: {p:?}");
            // The floor, or the top of the wall (2 m up).
            let top = (p[4] - 2.0).abs() < 0.05;
            assert!(p[4].abs() < 0.05 || top, "{p:?}");
            for &(to, a, b, link) in &edges[i] {
                let other = polys[to as usize];
                if link {
                    assert!(
                        top || (other[4] - 2.0).abs() < 0.05,
                        "jumps are off the wall"
                    );
                } else {
                    // A shared edge goes both ways, at one height.
                    assert!(edges[to as usize].iter().any(|e| e.0 as usize == i));
                    assert!((other[4] - p[4]).abs() < 0.05);
                }
                assert!(a.is_finite() && b.is_finite());
            }
        }
        assert!(
            edges.iter().all(|e| !e.is_empty()),
            "every polygon leads somewhere"
        );
    }

    #[test]
    fn off_the_mesh_it_moves_freely_back_on() {
        let mesh = floor_with_wall();
        // Beside the floor's eroded edge (x = 6 - radius).
        let from = Vec3::new(6.2, 0.0, -4.0);
        let to = mesh.move_along(from, Vec3::new(6.0, 0.0, -4.0));
        assert!((to.x - 6.0).abs() < 1e-4, "{to}");
        let on = mesh.move_along(to, Vec3::new(5.0, 0.0, -4.0));
        assert!((on.x - 5.0).abs() < 1e-4, "{on}");
    }

    #[test]
    fn a_straight_way_is_one_segment() {
        let mesh = floor_with_wall();
        let path = mesh
            .find_path(Vec3::new(-4.0, 0.0, -4.0), Vec3::new(-2.0, 0.0, 2.0), false)
            .unwrap();
        assert_eq!(path.len(), 2, "{path:?}");
        assert!(path[0].position.distance(Vec3::new(-4.0, 0.0, -4.0)) < 1e-3);
        assert!(path[1].position.distance(Vec3::new(-2.0, 0.0, 2.0)) < 1e-3);
    }

    #[test]
    fn paths_go_round_walls_hugging_the_corners() {
        let mesh = floor_with_wall();
        let from = Vec3::new(-3.0, 1.0, -3.0); // a character's middle
        let to = Vec3::new(3.0, 0.0, -3.0);
        let path = mesh.find_path(from, to, false).unwrap();
        assert!(path.len() >= 3, "{path:?}");
        // Round the wall's free end (z = 3, plus the radius) ...
        let max_z = path.iter().map(|p| p.position.z).fold(f32::MIN, f32::max);
        assert!((3.3..3.8).contains(&max_z), "{path:?}");
        // ... never through it, and tight: close to the shortest way.
        for w in path.windows(2) {
            let (a, b) = (w[0].position, w[1].position);
            for k in 0..=20 {
                let p = a.lerp(b, k as f32 / 20.0);
                assert!(
                    !(p.x.abs() < 0.8 && p.z < 3.3),
                    "{p} is in the wall: {path:?}"
                );
            }
        }
        // Up to the wall's end, across it, and back down.
        let shortest = 2.0 * Vec2::new(3.0 - 0.85, 6.35).length() + 1.7;
        assert!(length(&path) < shortest + 0.6, "{} {path:?}", length(&path));
        assert!(path.iter().all(|p| p.position.y.abs() < 1e-3 && !p.link));
    }

    #[test]
    fn unreachable_goals_give_none_or_the_nearest_way() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-4.0, -1.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        add_box(
            &mut v,
            &mut t,
            Vec3::new(6.0, -1.0, -2.0),
            Vec3::new(10.0, 0.0, 2.0),
        );
        let mesh = bake(&v, &t, &[], NavSettings::default()).unwrap();
        let (a, b) = (Vec3::new(-2.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0));
        assert!(mesh.find_path(a, b, false).is_none());
        let part = mesh.find_path(a, b, true).unwrap();
        let end = part.last().unwrap().position;
        assert!((end.x - -0.4).abs() < 0.11 && end.z.abs() < 0.2, "{part:?}");
        // Far off the mesh: nothing to start from.
        assert!(mesh
            .find_path(Vec3::new(-2.0, 50.0, 0.0), a, true)
            .is_none());
    }

    #[test]
    fn links_join_islands_and_the_path_says_where_to_jump() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-4.0, -1.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        add_box(
            &mut v,
            &mut t,
            Vec3::new(1.0, -1.0, -2.0),
            Vec3::new(5.0, 0.0, 2.0),
        );
        let mesh = bake(&v, &t, &[], NavSettings::default()).unwrap();
        let path = mesh
            .find_path(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(4.0, 0.0, -1.0), false)
            .unwrap();
        let jumps: Vec<_> = path.iter().filter(|p| p.link).collect();
        assert_eq!(jumps.len(), 1, "{path:?}");
        assert!(jumps[0].position.x > 1.0);
        // And back: the jump is two-way.
        assert!(mesh
            .find_path(Vec3::new(4.0, 0.0, -1.0), Vec3::new(-3.0, 0.0, 1.0), false)
            .is_some());

        // A hand-placed link instead of the automatic ones.
        let s = NavSettings {
            jump_distance: 0.0,
            ..NavSettings::default()
        };
        let one_way = Link {
            start: Vec3::new(-0.5, 0.0, 0.0),
            end: Vec3::new(1.5, 0.0, 0.0),
            two_way: false,
            auto: false,
        };
        let mesh = bake(&v, &t, &[one_way], s).unwrap();
        let there = mesh
            .find_path(Vec3::new(-3.0, 0.0, 1.0), Vec3::new(4.0, 0.0, -1.0), false)
            .unwrap();
        assert_eq!(there.iter().filter(|p| p.link).count(), 1);
        assert!(mesh
            .find_path(Vec3::new(4.0, 0.0, -1.0), Vec3::new(-3.0, 0.0, 1.0), false)
            .is_none());
    }

    #[test]
    fn stairs_and_ramps_carry_paths_up() {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        add_box(
            &mut v,
            &mut t,
            Vec3::new(-6.0, -1.0, -2.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        for k in 0..4 {
            let x = k as f32 * 0.5;
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
            Vec3::new(2.0, -1.0, -2.0),
            Vec3::new(6.0, 1.2, 2.0),
        );
        let mesh = bake(&v, &t, &[], NavSettings::default()).unwrap();
        let path = mesh
            .find_path(Vec3::new(-4.0, 0.0, 0.0), Vec3::new(4.0, 1.2, 0.0), false)
            .unwrap();
        let end = path.last().unwrap().position;
        assert!((end.y - 1.2).abs() < 1e-3, "{path:?}");
        assert!(path.iter().all(|p| !p.link));
        assert!((mesh.height_at(Vec3::new(4.0, 2.0, 0.0)).unwrap() - 1.2).abs() < 1e-3);
        assert!(mesh.height_at(Vec3::new(4.0, -5.0, 0.0)).is_none());
    }

    #[test]
    fn moving_along_stays_on_the_mesh() {
        let mesh = floor_with_wall();
        let from = Vec3::new(-2.0, 0.0, 0.0);
        // Straight into the wall: slides along it instead.
        let to = mesh.move_along(from, Vec3::new(0.0, 0.0, 0.5));
        assert!(to.x < -0.8 && (to.z - 0.5).abs() < 1e-4, "{to}");
        assert_eq!(mesh.move_along(from, Vec3::new(-1.5, 0.0, 0.2)).z, 0.2);
    }

    #[test]
    fn meshes_round_trip_through_ron() {
        let mesh = floor_with_wall();
        let text = mesh.to_ron().unwrap();
        let back = NavMesh::from_ron(&text).unwrap();
        assert_eq!(back, mesh);
        let a = Vec3::new(-3.0, 0.0, -3.0);
        let b = Vec3::new(3.0, 0.0, -3.0);
        assert_eq!(back.find_path(a, b, false), mesh.find_path(a, b, false));
        assert!(NavMesh::from_ron("(nope)").is_err());
    }
}
