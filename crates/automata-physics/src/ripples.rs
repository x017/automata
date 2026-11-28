//! Ripples: a small wave simulation on top of the water's Gerstner swell.
//!
//! A [`RippleField`] is a 256² heightfield, a quarter metre per cell (64 m
//! across), that follows the camera ([`RippleField::follow`]) in whole
//! cells. Things that cross the water surface push it about:
//!
//! - every body in the water carries a **dent** (the water it displaces:
//!   its footprint, as deep as it sits). Each step the field loses the
//!   dent where the body is now and gets back the one where it was, so a
//!   body that drops in sends out a ring, one that bobs sends out rings as
//!   it bobs, and one that moves sends ripples ahead and fills in behind.
//!   Moving faster than the ripples travel ([`SPEED`]), that makes the
//!   V-shaped wake of a boat (a Mach cone, close to the 19.5° of a real
//!   Kelvin wake at the speeds boats go);
//! - [`RippleField::splash`] (`ctx.water.splash` for scripts) drops a dent
//!   in on its own.
//!
//! The field is the linear wave equation, `h'' = c² ∇²h`, with a little
//! damping (rings fade over ~10 s), a little viscosity (no grid-scale
//! noise), and a sponge along the edges so rings leave the field instead
//! of bouncing back. Disturbances also whip up **foam**, which fades over
//! a few seconds (the white trail behind a boat).
//!
//! # Cost
//!
//! Nothing until something disturbs the water, and nothing again once the
//! field has calmed down. Running: about 0.25 ms per step in release on a
//! modest core (65 536 cells, a dozen multiply-adds each; debug builds are
//! some 40× slower), and the renderer gets a fresh [`RippleMap`] (packing
//! the cells as half floats) only when it's asked for one after a step.

use automata_renderer::{RippleMap, RIPPLE_FIELD_SIZE};
use glam::{Vec2, Vec3};

/// Metres per cell.
pub const CELL: f32 = 0.25;

/// How fast ripples spread (m/s): the speed of gravity waves about a metre
/// and a half long.
pub const SPEED: f32 = 1.6;

/// Velocity lost per second (rings fade over ~10 s).
const DAMPING: f32 = 0.3;

/// Viscosity (m²/s): smooths away what the grid can't represent.
const VISCOSITY: f32 = 0.004;

/// Width of the edge sponge (cells) and its damping at the very edge (/s).
const SPONGE: usize = 20;
const SPONGE_DAMPING: f32 = 6.0;

/// Foam lost per second; how fast (m/s) a dent must move the water to
/// whip any up, and foam per m/s beyond that, per step.
const FOAM_FADE: f32 = 0.45;
const FOAM_SPEED: f32 = 0.45;
const FOAM_PER_SPEED: f32 = 0.05;

/// How much of a body's draft becomes its dent (all of it: the water it
/// displaces), and the deepest dent (m).
const DENT_SCALE: f32 = 1.0;
const MAX_DENT: f32 = 0.6;

/// The narrowest dent (m): four cells.
const MIN_RADIUS: f32 = 4.0 * CELL;

/// Below this (m and m/s) everywhere, the field is calm and stops.
const CALM: f32 = 2e-4;

/// The water a body pushes aside: where (world XZ), how wide, how deep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dent {
    /// World XZ of the centre.
    pub center: Vec2,
    /// Radius (m).
    pub radius: f32,
    /// Depth (m) at the centre; it eases out to nothing at the radius.
    pub depth: f32,
}

impl Dent {
    /// The dent of a body whose bounding box has centre `center` and half
    /// size `half` (world axes), sitting in water whose surface is at
    /// `level` there.
    #[must_use]
    pub fn of_body(center: Vec3, half: Vec3, level: f32) -> Self {
        let draft = (level - (center.y - half.y)).clamp(0.0, 2.0 * half.y);
        Self {
            center: Vec2::new(center.x, center.z),
            // At least a few cells wide: narrower bumps are grid noise,
            // which a grid can't carry away (it would ring in place).
            // A circle of about the body's waterplane area.
            radius: ((half.x * half.z).sqrt() + 0.3).clamp(MIN_RADIUS, 6.0),
            depth: (draft * DENT_SCALE).min(MAX_DENT),
        }
    }

    /// The dent a splash of `strength` makes at `position`: 1 is about a
    /// crate dropped from a couple of metres.
    #[must_use]
    pub fn splash(position: Vec3, strength: f32) -> Self {
        let strength = strength.clamp(0.0, 10.0);
        Self {
            center: Vec2::new(position.x, position.z),
            radius: (0.5 + 0.5 * strength.sqrt()).clamp(MIN_RADIUS, 3.0),
            depth: (0.25 * strength).min(MAX_DENT * 2.0),
        }
    }
}

/// The ripple simulation around the camera.
#[derive(Debug)]
pub struct RippleField {
    /// Height (m) per cell, rows along +Z.
    height: Vec<f32>,
    /// Vertical speed (m/s) per cell.
    speed: Vec<f32>,
    /// Foam (0..1) per cell.
    foam: Vec<f32>,
    /// Scratch for the next speeds.
    next: Vec<f32>,
    /// Extra damping per cell from the edge sponge (/s).
    sponge: Vec<f32>,
    /// Speed kept per cell per substep (the damping, folded for `keep_dt`).
    keep: Vec<f32>,
    keep_dt: f32,
    /// The field's centre, in whole cells (world XZ / CELL).
    origin: [i32; 2],
    /// Where the camera would like the field.
    focus: Vec2,
    /// Disturbances to add in the next step: dents to dig, then to fill.
    pending: Vec<(Dent, f32)>,
    /// Something is moving (false: the field is flat and steps are free).
    active: bool,
    /// Bumped whenever the field changes.
    version: u64,
    /// The last snapshot handed out, and the version it shows.
    snapshot: Option<RippleMap>,
}

impl Default for RippleField {
    fn default() -> Self {
        Self::new()
    }
}

impl RippleField {
    /// Texels per side.
    pub const SIZE: usize = RIPPLE_FIELD_SIZE as usize;

    /// A flat field centred on the world origin.
    #[must_use]
    pub fn new() -> Self {
        let n = Self::SIZE;
        let mut sponge = vec![0.0; n * n];
        for z in 0..n {
            for x in 0..n {
                let edge = x.min(z).min(n - 1 - x).min(n - 1 - z);
                if edge < SPONGE {
                    #[allow(clippy::cast_precision_loss)] // small
                    let t = 1.0 - edge as f32 / SPONGE as f32;
                    sponge[z * n + x] = SPONGE_DAMPING * t * t;
                }
            }
        }
        Self {
            height: vec![0.0; n * n],
            speed: vec![0.0; n * n],
            foam: vec![0.0; n * n],
            next: vec![0.0; n * n],
            sponge,
            keep: vec![1.0; n * n],
            keep_dt: 0.0,
            origin: [0, 0],
            focus: Vec2::ZERO,
            pending: Vec::new(),
            active: false,
            version: 0,
            snapshot: None,
        }
    }

    /// Keeps the field where a camera at `eye` looking at `target` sees it:
    /// centred a little ahead of the eye, along the view (so more of it is
    /// in front of the camera than behind). Takes effect next step.
    pub fn follow(&mut self, eye: Vec3, target: Vec3) {
        let forward = (target - eye).normalize_or_zero();
        // Looking straight down, the field centres under the eye; looking
        // level, it reaches 0.3 of its width ahead.
        #[allow(clippy::cast_precision_loss)]
        let ahead = Self::SIZE as f32 * CELL * 0.3;
        self.focus = Vec2::new(eye.x + forward.x * ahead, eye.z + forward.z * ahead);
    }

    /// World XZ of the field's centre.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.origin[0] as f32, self.origin[1] as f32) * CELL
    }

    /// Adds `dent` (dug into the water) in the next step.
    pub fn dig(&mut self, dent: Dent) {
        self.pending.push((dent, -1.0));
    }

    /// Takes `dent` back out (the water fills in) in the next step.
    pub fn fill(&mut self, dent: Dent) {
        self.pending.push((dent, 1.0));
    }

    /// A splash of `strength` at `position` (see [`Dent::splash`]).
    pub fn splash(&mut self, position: Vec3, strength: f32) {
        if strength > 0.0 {
            self.dig(Dent::splash(position, strength));
        }
    }

    /// Flattens the water and forgets pending disturbances.
    pub fn calm(&mut self) {
        self.pending.clear();
        if self.active {
            self.height.fill(0.0);
            self.speed.fill(0.0);
            self.foam.fill(0.0);
            self.active = false;
            self.version += 1;
        }
    }

    /// Whether anything is moving.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The ripple height (m) at world `(x, z)`, or 0 off the field.
    #[must_use]
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        self.cell_at(Vec2::new(x, z))
            .map_or(0.0, |(cx, cz)| self.height[cz * Self::SIZE + cx])
    }

    /// The field's cell containing world `xz`, if any.
    fn cell_at(&self, xz: Vec2) -> Option<(usize, usize)> {
        let (corner, _) = self.extent();
        let local = (xz - corner) / CELL;
        #[allow(clippy::cast_precision_loss)]
        let n = Self::SIZE as f32;
        if local.x < 0.0 || local.y < 0.0 || local.x >= n || local.y >= n {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some((local.x as usize, local.y as usize))
    }

    /// World XZ of the field's corner and its width (m).
    fn extent(&self) -> (Vec2, f32) {
        #[allow(clippy::cast_precision_loss)]
        let width = Self::SIZE as f32 * CELL;
        (self.center() - Vec2::splat(width * 0.5), width)
    }

    /// Advances the field by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.recenter();
        self.apply_pending(dt);
        if !self.active || dt <= 0.0 {
            return;
        }
        // Explicit integration is stable while a ripple crosses well under
        // a cell per substep.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let substeps = ((dt * SPEED / (CELL * 0.35)).ceil() as usize).clamp(1, 8);
        #[allow(clippy::cast_precision_loss)]
        let h = dt / substeps as f32;
        for _ in 0..substeps {
            self.integrate(h);
        }
        let fade = (-FOAM_FADE * dt).exp();
        let mut peak = 0.0_f32;
        for ((foam, height), speed) in self.foam.iter_mut().zip(&self.height).zip(&self.speed) {
            *foam *= fade;
            peak = peak.max(height.abs()).max(speed.abs()).max(*foam);
        }
        self.version += 1;
        if peak < CALM {
            self.height.fill(0.0);
            self.speed.fill(0.0);
            self.foam.fill(0.0);
            self.active = false;
        }
    }

    /// One explicit step of the damped wave equation.
    fn integrate(&mut self, dt: f32) {
        let n = Self::SIZE;
        #[allow(clippy::float_cmp)] // the same substep as last time, exactly
        if self.keep_dt != dt {
            self.keep_dt = dt;
            for (keep, sponge) in self.keep.iter_mut().zip(&self.sponge) {
                *keep = 1.0 / (1.0 + (DAMPING + sponge) * dt);
            }
        }
        let c2 = SPEED * SPEED / (CELL * CELL) * dt;
        let nu = VISCOSITY / (CELL * CELL) * dt;
        let (height, speed, keep, next) = (&self.height, &self.speed, &self.keep, &mut self.next);
        for z in 0..n {
            let row = z * n;
            let up = if z > 0 { row - n } else { row };
            let down = if z + 1 < n { row + n } else { row };
            for x in 0..n {
                let i = row + x;
                let left = if x > 0 { i - 1 } else { i };
                let right = if x + 1 < n { i + 1 } else { i };
                let lap_h = height[left] + height[right] + height[up + x] + height[down + x]
                    - 4.0 * height[i];
                let lap_v =
                    speed[left] + speed[right] + speed[up + x] + speed[down + x] - 4.0 * speed[i];
                next[i] = (speed[i] + c2 * lap_h + nu * lap_v) * keep[i];
            }
        }
        std::mem::swap(&mut self.speed, &mut self.next);
        // Values too small to see become zero: subnormal floats would
        // slow the arithmetic several times over as the rings die out.
        for (h, v) in self.height.iter_mut().zip(self.speed.iter_mut()) {
            if v.abs() < 1e-7 {
                *v = 0.0;
            }
            let next = (*h + *v * dt).clamp(-1.0, 1.0);
            *h = if next.abs() < 1e-7 { 0.0 } else { next };
        }
    }

    /// Moves the field to the focus, in whole cells; the water that scrolls
    /// in is calm.
    fn recenter(&mut self) {
        #[allow(clippy::cast_possible_truncation)]
        let want = [
            (self.focus.x / CELL).round() as i32,
            (self.focus.y / CELL).round() as i32,
        ];
        let shift = [want[0] - self.origin[0], want[1] - self.origin[1]];
        if shift == [0, 0] {
            return;
        }
        self.origin = want;
        if !self.active {
            return;
        }
        let n = Self::SIZE;
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let size = n as i32;
        if shift[0].abs() >= size || shift[1].abs() >= size {
            self.height.fill(0.0);
            self.speed.fill(0.0);
            self.foam.fill(0.0);
            self.version += 1;
            return;
        }
        for field in [&mut self.height, &mut self.speed, &mut self.foam] {
            let next = &mut self.next;
            for z in 0..size {
                for x in 0..size {
                    let (sx, sz) = (x + shift[0], z + shift[1]);
                    #[allow(clippy::cast_sign_loss)]
                    let value = if (0..size).contains(&sx) && (0..size).contains(&sz) {
                        field[sz as usize * n + sx as usize]
                    } else {
                        0.0
                    };
                    #[allow(clippy::cast_sign_loss)]
                    {
                        next[z as usize * n + x as usize] = value;
                    }
                }
            }
            std::mem::swap(field, next);
        }
        self.version += 1;
    }

    /// Adds the pending dents to the heights, with foam where they moved
    /// the water (a body that bobs or moves whips some up; one at rest,
    /// whose dent is dug and filled in the same place, none).
    fn apply_pending(&mut self, dt: f32) {
        let pending = std::mem::take(&mut self.pending);
        let n = Self::SIZE;
        let (corner, _) = self.extent();
        // Each dent in field cells: centre, radius, and the cells it covers.
        #[allow(clippy::cast_precision_loss)]
        let last = Vec2::splat((n - 1) as f32);
        let dents: Vec<(Vec2, f32, f32, [usize; 4])> = pending
            .iter()
            .filter(|(dent, _)| dent.depth > 0.0 && dent.radius > 0.0)
            .filter_map(|(dent, sign)| {
                let local = (dent.center - corner) / CELL - Vec2::splat(0.5);
                let r = dent.radius / CELL;
                let lo = (local - Vec2::splat(r)).floor().max(Vec2::ZERO);
                let hi = (local + Vec2::splat(r)).ceil().min(last);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                (lo.x <= hi.x && lo.y <= hi.y).then(|| {
                    let rect = [lo.x as usize, hi.x as usize, lo.y as usize, hi.y as usize];
                    (local, r, sign * dent.depth, rect)
                })
            })
            .collect();
        let Some([x0, x1, z0, z1]) = dents.iter().map(|d| d.3).reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].max(b[1]),
                a[2].min(b[2]),
                a[3].max(b[3]),
            ]
        }) else {
            return;
        };
        // Sum them in the scratch buffer, then apply the net change.
        for z in z0..=z1 {
            self.next[z * n + x0..=z * n + x1].fill(0.0);
        }
        for (local, r, depth, [ax, bx, az, bz]) in dents {
            for z in az..=bz {
                for x in ax..=bx {
                    #[allow(clippy::cast_precision_loss)]
                    let d = Vec2::new(x as f32 - local.x, z as f32 - local.y).length() / r;
                    if d < 1.0 {
                        let weight = 0.5 + 0.5 * (std::f32::consts::PI * d).cos();
                        self.next[z * n + x] += depth * weight;
                    }
                }
            }
        }
        let mut moved = 0.0_f32;
        for z in z0..=z1 {
            for x in x0..=x1 {
                let i = z * n + x;
                let delta = self.next[i];
                self.height[i] = (self.height[i] + delta).clamp(-1.0, 1.0);
                // How fast the water moved (a splash: all at once).
                let speed = delta.abs() / dt.max(1.0 / 60.0);
                let foam = (speed - FOAM_SPEED).max(0.0) * FOAM_PER_SPEED;
                self.foam[i] = (self.foam[i] + foam).min(1.0);
                moved = moved.max(delta.abs());
            }
        }
        if moved > CALM * 0.1 {
            self.active = true;
            self.version += 1;
        }
    }

    /// The field as the renderer draws it, or `None` while it's calm.
    /// Cheap when nothing changed since the last call (the snapshot is
    /// shared); otherwise it packs the 256² texels.
    pub fn snapshot(&mut self) -> Option<RippleMap> {
        if !self.active {
            return None;
        }
        if self.snapshot.as_ref().map(|s| s.version) != Some(self.version) {
            #[allow(clippy::cast_possible_truncation)]
            let size = Self::SIZE as u32;
            self.snapshot = Some(RippleMap::new(
                self.center(),
                CELL,
                size,
                &self.height,
                &self.foam,
                self.version,
            ));
        }
        self.snapshot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_radius(field: &RippleField, center: Vec2) -> f32 {
        // The distance of the highest crest from `center`, along +X.
        let mut best = (0.0, f32::MIN);
        for i in 1..100 {
            #[allow(clippy::cast_precision_loss)]
            let d = i as f32 * CELL;
            let h = field.height_at(center.x + d, center.y);
            if h > best.1 {
                best = (d, h);
            }
        }
        best.0
    }

    #[test]
    fn a_splash_spreads_as_a_ring_at_the_ripple_speed() {
        let mut field = RippleField::new();
        assert!(field.snapshot().is_none(), "calm water has nothing to draw");
        // At a cell's centre, so the ring is symmetric on the grid.
        field.splash(Vec3::new(2.125, 0.0, -2.875), 1.0);
        let dt = 1.0 / 60.0;
        let mut radii = Vec::new();
        for _ in 0..3 {
            for _ in 0..60 {
                field.step(dt);
            }
            radii.push(ring_radius(&field, Vec2::new(2.125, -2.875)));
        }
        // The crest moves out at about 1.6 m/s.
        for pair in radii.windows(2) {
            let speed = pair[1] - pair[0];
            assert!(
                (1.2..2.0).contains(&speed),
                "rings spread at {speed} m/s: {radii:?}"
            );
        }
        let r = radii[2];
        // Symmetric: the same crest along -X.
        let east = field.height_at(2.125 + r, -2.875);
        let west = field.height_at(2.125 - r, -2.875);
        assert!((east - west).abs() < 1e-3 && east > 0.0, "{east} vs {west}");
        let map = field.snapshot().expect("rippling water");
        assert_eq!(map.texels.len(), RippleField::SIZE * RippleField::SIZE * 4);
    }

    #[test]
    fn rings_die_down_and_the_field_goes_quiet() {
        let mut field = RippleField::new();
        field.splash(Vec3::ZERO, 2.0);
        let mut steps = 0;
        while field.is_active() {
            field.step(1.0 / 30.0);
            steps += 1;
            assert!(steps < 30 * 120, "still rippling after two minutes");
        }
        assert!(field.snapshot().is_none());
        assert_eq!(field.height_at(0.5, 0.0), 0.0);
    }

    #[test]
    fn a_dent_that_stays_put_makes_no_waves() {
        let mut field = RippleField::new();
        let dent = Dent::of_body(Vec3::new(0.0, 0.0, 0.0), Vec3::splat(0.5), 0.2);
        field.dig(dent);
        field.fill(dent);
        field.step(1.0 / 60.0);
        assert!(field.height_at(0.0, 0.0).abs() < 1e-6);
    }

    #[test]
    fn a_fast_boat_leaves_a_v_shaped_wake() {
        let mut field = RippleField::new();
        let half = Vec3::new(0.8, 0.4, 0.8);
        let speed = 5.0;
        let dt = 1.0 / 60.0;
        let mut previous: Option<Dent> = None;
        let mut x = -20.0;
        // Five seconds.
        for _ in 0..300 {
            x += speed * dt;
            let dent = Dent::of_body(Vec3::new(x, 0.0, 0.0), half, 0.2);
            if let Some(p) = previous {
                field.fill(p);
            }
            field.dig(dent);
            previous = Some(dent);
            field.step(dt);
        }
        // Behind the boat, the wake's arms spread at the Mach angle
        // asin(c / v) ≈ 18.7° each side; ahead it's calm.
        let behind = 8.0;
        let angle = (SPEED / speed).asin();
        let arm = behind * angle.tan();
        let energy = |dz: f32| {
            (-4..=4)
                .map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let z = dz + i as f32 * CELL * 0.5;
                    field.height_at(x - behind, z).abs()
                })
                .sum::<f32>()
        };
        let on_arm = energy(arm).min(energy(-arm));
        let outside = energy(arm * 2.5).max(energy(-arm * 2.5));
        assert!(on_arm > outside * 3.0, "arm {on_arm} vs outside {outside}");
        let ahead = (0..10)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let d = 3.0 + i as f32;
                field.height_at(x + d, 0.0).abs()
            })
            .fold(0.0, f32::max);
        assert!(ahead < 1e-3, "{ahead} m of ripple ahead of the boat");
        // And foam trails behind it.
        let map = field.snapshot().unwrap();
        assert!(map.texels.iter().skip(3).step_by(4).any(|&f| f > 0x3000));
    }

    #[test]
    fn the_field_follows_the_view_and_keeps_its_ripples_in_place() {
        let mut field = RippleField::new();
        field.splash(Vec3::new(4.0, 0.0, 4.0), 1.0);
        field.step(1.0 / 60.0);
        let before = field.height_at(4.0, 4.0);
        assert!(before < 0.0);
        // Look along +Z from (4, 10, 0): the field moves ahead of the eye.
        field.follow(Vec3::new(4.0, 10.0, 0.0), Vec3::new(4.0, 0.0, 20.0));
        field.step(0.0);
        let c = field.center();
        assert!((c.x - 4.0).abs() < CELL && c.y > 4.0, "{c}");
        assert!((field.height_at(4.0, 4.0) - before).abs() < 1e-6);
    }
}
