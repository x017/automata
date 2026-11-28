//! Navigation for AUTOMATA: where characters can walk, and how to get
//! from here to there.
//!
//! - **Baking** ([`bake`]) turns the level's static colliders into a
//!   navigation mesh: the triangles are voxelised into columns of solid
//!   spans, the tops an agent fits on (not too steep, enough headroom)
//!   become walkable cells, cells within the agent's radius of a wall or a
//!   ledge are eroded away, the rest are flood-filled into regions and
//!   merged into flat rectangles: the mesh's polygons. Gaps an agent could
//!   jump across or drop down become off-mesh links, and [`NavLink`]s add
//!   more by hand.
//! - **Queries** ([`NavMesh::find_path`]) run A* over the polygons and
//!   pull the result tight with the funnel algorithm, so paths hug corners
//!   instead of zig-zagging through cell centres.
//! - **Agents** ([`NavAgent`], [`step_agents`]) follow paths to a point or
//!   an entity, steer and keep apart from each other, and jump links.
//!
//! The baked mesh is plain data (RON), saved next to the scene by the
//! editor's Bake button; a game whose level changed since bakes it again
//! when it starts.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod agent;
mod bake;
mod query;
pub mod scene;

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub use agent::{step_agents, AgentState, AgentWorld, NavAgent, NavEdit, NavTarget};
pub use bake::{bake, geometry_hash, BakeError};
pub use query::PathPoint;

/// How a navigation mesh is baked: the size of the agents it's for and
/// how fine the voxel grid is. Part of the scene's [`Navigation`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavSettings {
    /// Width of a voxel column in metres. Smaller is more exact and slower
    /// to bake.
    pub cell_size: f32,
    /// Height steps closer than this merge (metres).
    pub cell_height: f32,
    /// How far agents keep from walls and ledges (metres).
    pub agent_radius: f32,
    /// The headroom an agent needs (metres).
    pub agent_height: f32,
    /// The highest step an agent walks up (metres).
    pub max_climb: f32,
    /// The steepest slope an agent walks up (degrees).
    pub max_slope: f32,
    /// How far across a gap agents jump (metres, edge to edge of the
    /// walkable areas); 0 makes no jump links.
    pub jump_distance: f32,
    /// How much higher a jump may land (metres).
    pub jump_height: f32,
    /// How far down agents drop (metres).
    pub drop_height: f32,
    /// Walkable patches smaller than this (square metres) are left out.
    pub min_area: f32,
}

impl Default for NavSettings {
    fn default() -> Self {
        Self {
            cell_size: 0.2,
            cell_height: 0.1,
            agent_radius: 0.35,
            agent_height: 1.8,
            max_climb: 0.4,
            max_slope: 45.0,
            jump_distance: 2.5,
            jump_height: 1.2,
            drop_height: 3.0,
            min_area: 0.5,
        }
    }
}

impl NavSettings {
    /// The settings with every value clamped into a sane range (a cell
    /// can't be smaller than 2 cm, an agent can't be negative …).
    #[must_use]
    pub fn sanitized(self) -> Self {
        let pos = |v: f32, min: f32, max: f32, or: f32| {
            if v.is_finite() {
                v.clamp(min, max)
            } else {
                or
            }
        };
        let d = Self::default();
        Self {
            cell_size: pos(self.cell_size, 0.02, 4.0, d.cell_size),
            cell_height: pos(self.cell_height, 0.01, 2.0, d.cell_height),
            agent_radius: pos(self.agent_radius, 0.0, 10.0, d.agent_radius),
            agent_height: pos(self.agent_height, 0.05, 50.0, d.agent_height),
            max_climb: pos(self.max_climb, 0.0, 10.0, d.max_climb),
            max_slope: pos(self.max_slope, 0.0, 89.0, d.max_slope),
            jump_distance: pos(self.jump_distance, 0.0, 50.0, d.jump_distance),
            jump_height: pos(self.jump_height, 0.0, 50.0, d.jump_height),
            drop_height: pos(self.drop_height, 0.0, 100.0, d.drop_height),
            min_area: pos(self.min_area, 0.0, 1.0e6, d.min_area),
        }
    }
}

/// The scene's navigation (a component, usually on an entity called
/// "Navigation"): how its mesh is baked and the file it was baked to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Navigation {
    /// How the mesh is baked.
    pub settings: NavSettings,
    /// The baked mesh, relative to the scene's folder (`level.nav.ron`);
    /// empty until baked.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
}

/// A hand-placed off-mesh link (a component): agents may get from this
/// entity's position to the position of the entity named `to` (a jump, a
/// ladder, a teleporter), and back when `two_way`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavLink {
    /// The name of the entity at the other end.
    pub to: String,
    /// Whether agents may use it both ways.
    pub two_way: bool,
}

impl Default for NavLink {
    fn default() -> Self {
        Self {
            to: String::new(),
            two_way: true,
        }
    }
}

/// One polygon of a navigation mesh: a rectangle of grid cells
/// `[min, max)` lying on a plane.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Poly {
    /// The first cell (x, z).
    pub min: [u32; 2],
    /// One past the last cell (x, z).
    pub max: [u32; 2],
    /// The surface: height at the first cell's centre, then the rise per
    /// metre along x and along z.
    pub plane: [f32; 3],
    /// The connected walkable area it belongs to.
    pub region: u32,
}

/// An off-mesh link: a way between two points the mesh itself doesn't
/// join (a gap to jump, a drop, a hand-placed [`NavLink`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// Where it starts (on the mesh).
    pub start: Vec3,
    /// Where it lands (on the mesh).
    pub end: Vec3,
    /// Whether it can be taken from `end` back to `start` as well.
    pub two_way: bool,
    /// Whether the bake found it (a jump or drop) rather than a [`NavLink`].
    pub auto: bool,
}

/// A baked navigation mesh. Load one with [`NavMesh::from_ron`] or bake
/// one with [`bake`]; then ask it for paths with [`NavMesh::find_path`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavMesh {
    /// What it was baked with.
    pub settings: NavSettings,
    /// The grid's corner (x and z of cell (0, 0)'s low corner).
    pub origin: [f32; 2],
    /// Cells along x and z.
    pub size: [u32; 2],
    /// The walkable polygons.
    pub polys: Vec<Poly>,
    /// Jumps, drops and hand-placed links.
    pub links: Vec<Link>,
    /// Fingerprint of what it was baked from (see [`geometry_hash`]): a
    /// mismatch means the level changed since.
    pub hash: u64,
    /// How many separate walkable regions there are.
    #[serde(default)]
    pub regions: u32,
    #[serde(skip)]
    graph: query::Graph,
}

impl PartialEq for NavMesh {
    fn eq(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.origin == other.origin
            && self.size == other.size
            && self.polys == other.polys
            && self.links == other.links
            && self.hash == other.hash
    }
}

/// What can go wrong loading a baked mesh.
#[derive(Debug)]
pub enum LoadError {
    /// The file couldn't be read.
    Io(std::io::Error),
    /// The file isn't a navigation mesh.
    Parse(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "not a navigation mesh: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl NavMesh {
    /// A mesh from its parts, ready for queries.
    #[must_use]
    pub fn new(
        settings: NavSettings,
        origin: [f32; 2],
        size: [u32; 2],
        polys: Vec<Poly>,
        links: Vec<Link>,
        hash: u64,
    ) -> Self {
        let regions = polys.iter().map(|p| p.region + 1).max().unwrap_or(0);
        let mut mesh = Self {
            settings,
            origin,
            size,
            polys,
            links,
            hash,
            regions,
            graph: query::Graph::default(),
        };
        mesh.graph = query::Graph::build(&mesh);
        mesh
    }

    /// The mesh as RON text (what the editor saves).
    ///
    /// # Errors
    ///
    /// When RON can't write it (it always can).
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(
            self,
            ron::ser::PrettyConfig::default()
                .compact_arrays(true)
                .depth_limit(2),
        )
    }

    /// A mesh from RON text, ready for queries.
    ///
    /// # Errors
    ///
    /// [`LoadError::Parse`] when the text isn't a mesh.
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        let mut mesh: Self = ron::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        mesh.settings = mesh.settings.sanitized();
        if mesh.polys.iter().any(|p| {
            p.max[0] > mesh.size[0]
                || p.max[1] > mesh.size[1]
                || p.min[0] >= p.max[0]
                || p.min[1] >= p.max[1]
        }) {
            return Err(LoadError::Parse("a polygon lies outside the grid".into()));
        }
        mesh.regions = mesh.polys.iter().map(|p| p.region + 1).max().unwrap_or(0);
        mesh.graph = query::Graph::build(&mesh);
        Ok(mesh)
    }

    /// Reads a mesh saved with [`NavMesh::to_ron`].
    ///
    /// # Errors
    ///
    /// When the file can't be read or isn't a mesh.
    pub fn load(path: &std::path::Path) -> Result<Self, LoadError> {
        let text = std::fs::read_to_string(path).map_err(LoadError::Io)?;
        Self::from_ron(&text)
    }

    /// The world-space corners of polygon `i` (counter-clockwise seen
    /// from above), on its plane: for drawing the mesh.
    #[must_use]
    pub fn poly_corners(&self, i: usize) -> [Vec3; 4] {
        let p = &self.polys[i];
        let cs = self.settings.cell_size;
        let x0 = self.origin[0] + p.min[0] as f32 * cs;
        let z0 = self.origin[1] + p.min[1] as f32 * cs;
        let x1 = self.origin[0] + p.max[0] as f32 * cs;
        let z1 = self.origin[1] + p.max[1] as f32 * cs;
        let at = |x: f32, z: f32| Vec3::new(x, self.plane_height(i, x, z), z);
        [at(x0, z0), at(x0, z1), at(x1, z1), at(x1, z0)]
    }

    /// The height of polygon `i`'s plane at world `(x, z)`.
    #[must_use]
    pub fn plane_height(&self, i: usize, x: f32, z: f32) -> f32 {
        let p = &self.polys[i];
        let cs = self.settings.cell_size;
        let cx = self.origin[0] + (p.min[0] as f32 + 0.5) * cs;
        let cz = self.origin[1] + (p.min[1] as f32 + 0.5) * cs;
        p.plane[0] + p.plane[1] * (x - cx) + p.plane[2] * (z - cz)
    }

    /// The walkable area in square metres.
    #[must_use]
    pub fn area(&self) -> f32 {
        let cs = self.settings.cell_size;
        self.polys
            .iter()
            .map(|p| ((p.max[0] - p.min[0]) * (p.max[1] - p.min[1])) as f32 * cs * cs)
            .sum()
    }

    /// Whether the mesh has nowhere to walk.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.polys.is_empty()
    }
}
