//! Prefab links: entities that follow a prefab file.
//!
//! A prefab is a scene file (`.prefab`) holding the entities to stamp. An
//! entity placed from one can keep a [`PrefabLink`] to it. The scene still
//! stores the entity's full data, so it loads without the prefab. But
//! loading ([`SceneFile::resolve_prefabs`]) re-syncs it: every field comes
//! from the prefab again, except the fields the instance **overrides**.
//!
//! Overrides are field paths like `"mesh_visual.color"`,
//! `"attributes.speed"` or `"rigid_body"` (a whole component the instance
//! added or removed). [`overrides`] finds them by comparing an instance with
//! its prefab, field by field, so an editor never has to track them by
//! hand. The name, position and rotation always belong to the instance.
//!
//! A **variant** is a prefab whose entity itself links to another prefab:
//! it follows its base the same way and adds overrides of its own.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::scene_file::{EntityData, SceneFile};

/// Fields that always belong to the instance: never synced from the
/// prefab, never listed as overrides.
pub const INSTANCE_FIELDS: [&str; 4] = [
    "name",
    "transform.position",
    "transform.rotation_euler",
    "prefab",
];

/// How deep variants may chain (a variant of a variant of …).
const MAX_DEPTH: usize = 16;

/// An entity's link to the prefab it was placed from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefabLink {
    /// The `.prefab` file, relative to the asset root (the scene's folder),
    /// like `"prefabs/coin.prefab"`.
    pub path: String,
    /// Which of the prefab's entities this one follows (0 for one-entity
    /// prefabs).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub index: u32,
    /// The field paths this instance overrides, as of the last save.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes `&T`
fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl PrefabLink {
    /// A link to entity 0 of `path`, with no overrides.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            index: 0,
            overrides: Vec::new(),
        }
    }
}

// ----- the field tree ------------------------------------------------------

/// An entity's data as a tree of named fields, with everything else (numbers,
/// lists, enums) kept as RON text leaves.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    /// A value printed as RON.
    Leaf(String),
    /// A struct's fields, in order (fields serde skipped are absent).
    Struct(Vec<(String, Node)>),
    /// A map's entries: the key as RON text, then the value.
    Map(Vec<(String, Node)>),
    /// `Some(value)`.
    Some(Box<Node>),
    /// A newtype struct `(value)`.
    Newtype(Box<Node>),
}

impl Node {
    fn of<T: Serialize + ?Sized>(value: &T) -> Result<Self, String> {
        value.serialize(TreeSer).map_err(|e| e.0)
    }

    /// The node's fields, if it has any (looking through `Some`).
    fn entries(&self) -> Option<&Vec<(String, Node)>> {
        match self {
            Self::Struct(f) | Self::Map(f) => Some(f),
            Self::Some(inner) | Self::Newtype(inner) => inner.entries(),
            Self::Leaf(_) => None,
        }
    }

    fn entries_mut(&mut self) -> Option<&mut Vec<(String, Node)>> {
        match self {
            Self::Struct(f) | Self::Map(f) => Some(f),
            Self::Some(inner) | Self::Newtype(inner) => inner.entries_mut(),
            Self::Leaf(_) => None,
        }
    }

    fn print(&self, out: &mut String) {
        match self {
            Self::Leaf(text) => out.push_str(text),
            Self::Struct(fields) | Self::Map(fields) => {
                let (open, close) = if matches!(self, Self::Map(_)) {
                    ('{', '}')
                } else {
                    ('(', ')')
                };
                out.push(open);
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(k);
                    out.push(':');
                    v.print(out);
                }
                out.push(close);
            }
            Self::Some(inner) => {
                out.push_str("Some(");
                inner.print(out);
                out.push(')');
            }
            Self::Newtype(inner) => {
                out.push('(');
                inner.print(out);
                out.push(')');
            }
        }
    }
}

/// A key as a path segment: map keys that are strings lose their quotes.
fn key_name(raw: &str) -> std::borrow::Cow<'_, str> {
    if raw.starts_with('"') {
        if let Ok(s) = ron::from_str::<String>(raw) {
            return s.into();
        }
    }
    raw.into()
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// The paths where `a` and `b` differ (the shallowest one per difference).
fn diff(a: &Node, b: &Node, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Node::Some(x), Node::Some(y)) | (Node::Newtype(x), Node::Newtype(y)) => {
            diff(x, y, path, out);
        }
        (Node::Struct(fa), Node::Struct(fb)) | (Node::Map(fa), Node::Map(fb)) => {
            for (k, va) in fa {
                let name = key_name(k);
                match fb.iter().find(|(kb, _)| key_name(kb) == name) {
                    Some((_, vb)) => diff(va, vb, &join(path, &name), out),
                    None => out.push(join(path, &name)),
                }
            }
            for (k, _) in fb {
                let name = key_name(k);
                if !fa.iter().any(|(ka, _)| key_name(ka) == name) {
                    out.push(join(path, &name));
                }
            }
        }
        (Node::Leaf(x), Node::Leaf(y)) if x == y => {}
        _ => out.push(path.to_string()),
    }
}

/// Sets `onto` at `segments` to `from`'s value there (removing the field
/// when `from` lacks it).
fn copy_at(onto: &mut Node, from: &Node, segments: &[&str]) {
    let Some((first, rest)) = segments.split_first() else {
        *onto = from.clone();
        return;
    };
    let (Some(_), Some(src)) = (onto.entries(), from.entries()) else {
        // Shapes differ (a component one side lacks): take the whole value.
        *onto = from.clone();
        return;
    };
    let from_entry = src.iter().find(|(k, _)| key_name(k) == *first).cloned();
    let Some(dst) = onto.entries_mut() else {
        return;
    };
    let at = dst.iter().position(|(k, _)| key_name(k) == *first);
    match (at, from_entry) {
        (Some(i), Some((_, value))) => copy_at(&mut dst[i].1, &value, rest),
        (None, Some(entry)) => dst.push(entry),
        (Some(i), None) => {
            dst.remove(i);
        }
        (None, None) => {}
    }
}

fn is_instance_field(path: &str) -> bool {
    INSTANCE_FIELDS
        .iter()
        .any(|f| path == *f || path.strip_prefix(f).is_some_and(|r| r.starts_with('.')))
}

fn parse(tree: &Node) -> Result<EntityData, String> {
    let mut text = String::new();
    tree.print(&mut text);
    ron::from_str(&text).map_err(|e| format!("{e} in {text}"))
}

// ----- the public operations ----------------------------------------------

/// The fields where `instance` differs from its prefab entity `base`,
/// as paths like `"mesh_visual.color"`, without [`INSTANCE_FIELDS`].
#[must_use]
pub fn overrides(base: &EntityData, instance: &EntityData) -> Vec<String> {
    let (Ok(a), Ok(b)) = (Node::of(base), Node::of(instance)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    diff(&a, &b, "", &mut out);
    out.retain(|p| !is_instance_field(p));
    out
}

/// `onto` with `from`'s values at `paths` (fields `from` lacks are removed).
///
/// # Errors
///
/// When the result doesn't read back as entity data (a path through
/// mismatched shapes).
pub fn copy_paths(
    from: &EntityData,
    onto: &EntityData,
    paths: &[String],
) -> Result<EntityData, String> {
    let src = Node::of(from)?;
    let mut tree = Node::of(onto)?;
    for p in paths {
        let segments: Vec<&str> = p.split('.').collect();
        copy_at(&mut tree, &src, &segments);
    }
    parse(&tree)
}

/// The instance as its prefab entity `base` makes it: `base`'s fields,
/// except the instance's own [`INSTANCE_FIELDS`] and `overrides`.
///
/// # Errors
///
/// As [`copy_paths`].
pub fn apply(
    base: &EntityData,
    instance: &EntityData,
    overrides: &[String],
) -> Result<EntityData, String> {
    let mut paths: Vec<String> = INSTANCE_FIELDS.iter().map(|s| (*s).to_string()).collect();
    paths.extend(overrides.iter().cloned());
    copy_paths(instance, base, &paths)
}

/// Where a link's path points, under the asset `root`.
#[must_use]
pub fn resolve_path(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

/// The entity a link follows, fully resolved: read from its prefab file,
/// and, for a variant, made from its own base with the variant's
/// overrides. Its own link is cleared.
///
/// # Errors
///
/// A missing or unreadable file, a missing entity, or variants that loop.
pub fn load_base(link: &PrefabLink, root: &Path) -> Result<EntityData, String> {
    load_base_at(link, root, 0)
}

fn load_base_at(link: &PrefabLink, root: &Path, depth: usize) -> Result<EntityData, String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: variants chain too deep (a loop?)", link.path));
    }
    let file = resolve_path(root, &link.path);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", link.path))?;
    let scene = SceneFile::from_ron(&text).map_err(|e| format!("{}: {e}", link.path))?;
    let mut entity = scene
        .entities
        .into_iter()
        .nth(link.index as usize)
        .ok_or_else(|| format!("{} has no entity {}", link.path, link.index))?;
    if let Some(base_link) = entity.prefab.take() {
        let base = load_base_at(&base_link, root, depth + 1)?;
        entity = apply(&base, &entity, &base_link.overrides)?;
        entity.prefab = None;
    }
    Ok(entity)
}

impl SceneFile {
    /// Re-syncs every prefab instance with its prefab file (paths relative
    /// to `root`): each takes the prefab's fields except its own name,
    /// position, rotation and overrides. An instance whose prefab can't be
    /// read keeps its stored data. Returns one message per such problem.
    pub fn resolve_prefabs(&mut self, root: &Path) -> Vec<String> {
        let mut bases: HashMap<(String, u32), Result<EntityData, String>> = HashMap::new();
        let mut problems = Vec::new();
        for entity in &mut self.entities {
            let Some(link) = entity.prefab.clone() else {
                continue;
            };
            let base = bases
                .entry((link.path.clone(), link.index))
                .or_insert_with(|| load_base(&link, root));
            let name = entity.name.clone().unwrap_or_else(|| "(unnamed)".into());
            match base {
                Ok(base) => match apply(base, entity, &link.overrides) {
                    Ok(synced) => *entity = synced,
                    Err(e) => problems.push(format!("{name}: can't follow {}: {e}", link.path)),
                },
                Err(e) => problems.push(format!("{name}: prefab {e} (kept the scene's copy)")),
            }
        }
        problems
    }
}

/// A readable name for an override path: `"mesh_visual.color"` reads
/// "Mesh Renderer › Color".
#[must_use]
pub fn describe(path: &str) -> String {
    let mut out = String::new();
    for (i, seg) in path.split('.').enumerate() {
        if i > 0 {
            out.push_str(" › ");
        }
        let label = match (i, seg) {
            (0, "mesh_visual") => "Mesh Renderer".to_string(),
            (0, "collider_3d") => "Collider".to_string(),
            (0, "collider_2d") => "Collider 2D".to_string(),
            (0, "rigid_body") => "Rigidbody".to_string(),
            (0, "directional_light") => "Light".to_string(),
            (0, "camera_3d") => "Camera".to_string(),
            (_, "rotation_euler") => "Rotation".to_string(),
            (1, _) if path.starts_with("attributes.") => seg.to_string(),
            _ => {
                let mut word = String::new();
                for (j, part) in seg.split('_').filter(|p| !p.is_empty()).enumerate() {
                    if j > 0 {
                        word.push(' ');
                    }
                    let mut chars = part.chars();
                    if let Some(c) = chars.next() {
                        let _ = write!(word, "{}{}", c.to_uppercase(), chars.as_str());
                    }
                }
                word
            }
        };
        out.push_str(&label);
    }
    out
}

// ----- the serializer ------------------------------------------------------

#[derive(Debug)]
struct TreeErr(String);

impl std::fmt::Display for TreeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TreeErr {}

impl serde::ser::Error for TreeErr {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

fn ron_text<T: Serialize + ?Sized>(value: &T) -> Result<String, TreeErr> {
    ron::to_string(value).map_err(|e| TreeErr(e.to_string()))
}

/// Serializes into a [`Node`]: structs and maps stay trees, everything else
/// becomes a RON leaf.
struct TreeSer;

/// Collects the items of a list, tuple or enum variant as RON text.
struct ListSer {
    items: Vec<String>,
    open: String,
    close: char,
}

impl ListSer {
    fn new(open: impl Into<String>, close: char) -> Self {
        Self {
            items: Vec::new(),
            open: open.into(),
            close,
        }
    }

    fn finish(self) -> Node {
        let mut s = self.open;
        s.push_str(&self.items.join(","));
        s.push(self.close);
        Node::Leaf(s)
    }
}

/// Collects a struct's (or map's) fields as subtrees.
struct FieldsSer {
    fields: Vec<(String, Node)>,
    map: bool,
    key: Option<String>,
}

/// Collects a struct variant's fields as RON text.
struct VariantFieldsSer {
    variant: &'static str,
    fields: Vec<String>,
}

macro_rules! leaf {
    ($($name:ident: $ty:ty),* $(,)?) => {
        $(fn $name(self, v: $ty) -> Result<Node, TreeErr> {
            Ok(Node::Leaf(ron_text(&v)?))
        })*
    };
}

impl serde::Serializer for TreeSer {
    type Ok = Node;
    type Error = TreeErr;
    type SerializeSeq = ListSer;
    type SerializeTuple = ListSer;
    type SerializeTupleStruct = ListSer;
    type SerializeTupleVariant = ListSer;
    type SerializeMap = FieldsSer;
    type SerializeStruct = FieldsSer;
    type SerializeStructVariant = VariantFieldsSer;

    leaf!(
        serialize_bool: bool,
        serialize_i8: i8,
        serialize_i16: i16,
        serialize_i32: i32,
        serialize_i64: i64,
        serialize_u8: u8,
        serialize_u16: u16,
        serialize_u32: u32,
        serialize_u64: u64,
        serialize_f32: f32,
        serialize_f64: f64,
        serialize_char: char,
        serialize_str: &str,
        serialize_bytes: &[u8],
    );

    fn serialize_none(self) -> Result<Node, TreeErr> {
        Ok(Node::Leaf("None".into()))
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Node, TreeErr> {
        Ok(Node::Some(Box::new(value.serialize(TreeSer)?)))
    }

    fn serialize_unit(self) -> Result<Node, TreeErr> {
        Ok(Node::Leaf("()".into()))
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Node, TreeErr> {
        Ok(Node::Leaf("()".into()))
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Node, TreeErr> {
        Ok(Node::Leaf(variant.into()))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Node, TreeErr> {
        Ok(Node::Newtype(Box::new(value.serialize(TreeSer)?)))
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Node, TreeErr> {
        Ok(Node::Leaf(format!("{variant}({})", ron_text(value)?)))
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<ListSer, TreeErr> {
        Ok(ListSer::new("[", ']'))
    }

    fn serialize_tuple(self, _: usize) -> Result<ListSer, TreeErr> {
        Ok(ListSer::new("(", ')'))
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<ListSer, TreeErr> {
        Ok(ListSer::new("(", ')'))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<ListSer, TreeErr> {
        Ok(ListSer::new(format!("{variant}("), ')'))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<FieldsSer, TreeErr> {
        Ok(FieldsSer {
            fields: Vec::new(),
            map: true,
            key: None,
        })
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<FieldsSer, TreeErr> {
        Ok(FieldsSer {
            fields: Vec::new(),
            map: false,
            key: None,
        })
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<VariantFieldsSer, TreeErr> {
        Ok(VariantFieldsSer {
            variant,
            fields: Vec::new(),
        })
    }
}

impl serde::ser::SerializeSeq for ListSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TreeErr> {
        self.items.push(ron_text(value)?);
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeTuple for ListSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TreeErr> {
        self.items.push(ron_text(value)?);
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeTupleStruct for ListSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TreeErr> {
        self.items.push(ron_text(value)?);
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeTupleVariant for ListSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TreeErr> {
        self.items.push(ron_text(value)?);
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(self.finish())
    }
}

impl serde::ser::SerializeMap for FieldsSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), TreeErr> {
        self.key = Some(ron_text(key)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), TreeErr> {
        let key = self.key.take().unwrap_or_default();
        self.fields.push((key, value.serialize(TreeSer)?));
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(Node::Map(self.fields))
    }
}

impl serde::ser::SerializeStruct for FieldsSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), TreeErr> {
        self.fields
            .push((key.to_string(), value.serialize(TreeSer)?));
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(if self.map {
            Node::Map(self.fields)
        } else {
            Node::Struct(self.fields)
        })
    }
}

impl serde::ser::SerializeStructVariant for VariantFieldsSer {
    type Ok = Node;
    type Error = TreeErr;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), TreeErr> {
        self.fields.push(format!("{key}:{}", ron_text(value)?));
        Ok(())
    }
    fn end(self) -> Result<Node, TreeErr> {
        Ok(Node::Leaf(format!(
            "{}({})",
            self.variant,
            self.fields.join(",")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_file::{AttributeValue, MeshVisualData, PrimitiveData, TransformData};

    fn coin() -> EntityData {
        let mut data = EntityData {
            name: Some("Coin".into()),
            transform: Some(TransformData::default()),
            mesh_visual: Some(MeshVisualData {
                primitive: PrimitiveData::Sphere { size: 0.5 },
                color: [1.0, 0.8, 0.1, 1.0],
                surface: crate::scene_file::Surface::default(),
                draw_distance: 0.0,
                material: String::new(),
            }),
            ..EntityData::default()
        };
        data.attributes
            .insert("value".into(), AttributeValue::Int(1));
        data
    }

    #[test]
    fn every_entity_survives_the_tree_round_trip() {
        let mut data = coin();
        data.rigid_body = Some(automata_physics::RigidBody::dynamic());
        data.collider_3d = Some(automata_physics::Collider3D::ball(0.25));
        data.script = Some(crate::scene_file::Script {
            path: "scripts/spin.py".into(),
        });
        data.prefab = Some(PrefabLink {
            path: "prefabs/coin.prefab".into(),
            index: 2,
            overrides: vec!["mesh_visual.color".into()],
        });
        let tree = Node::of(&data).unwrap();
        assert_eq!(parse(&tree).unwrap(), data);
        assert_eq!(overrides(&data, &data), Vec::<String>::new());
    }

    #[test]
    fn overrides_are_the_changed_fields_and_instances_follow_the_rest() {
        let base = coin();
        let mut instance = base.clone();
        instance.name = Some("Coin 7".into());
        instance.transform.as_mut().unwrap().position = [3.0, 1.0, 0.0];
        instance.mesh_visual.as_mut().unwrap().color = [0.2, 0.9, 1.0, 1.0];
        instance
            .attributes
            .insert("value".into(), AttributeValue::Int(5));
        instance.rigid_body = Some(automata_physics::RigidBody::fixed());
        let ov = overrides(&base, &instance);
        assert_eq!(
            ov,
            ["mesh_visual.color", "attributes.value", "rigid_body"],
            "name and position belong to the instance"
        );

        // The prefab changes its size and colour and gains an attribute.
        let mut changed = base.clone();
        changed.mesh_visual.as_mut().unwrap().primitive = PrimitiveData::Cube { size: 0.4 };
        changed.mesh_visual.as_mut().unwrap().color = [1.0, 0.0, 0.0, 1.0];
        changed
            .attributes
            .insert("spin".into(), AttributeValue::Float(2.0));
        let synced = apply(&changed, &instance, &ov).unwrap();
        let mesh = synced.mesh_visual.as_ref().unwrap();
        assert_eq!(
            mesh.primitive,
            PrimitiveData::Cube { size: 0.4 },
            "followed"
        );
        assert_eq!(mesh.color, [0.2, 0.9, 1.0, 1.0], "kept its override");
        assert_eq!(synced.attributes["value"], AttributeValue::Int(5));
        assert_eq!(synced.attributes["spin"], AttributeValue::Float(2.0));
        assert!(synced.rigid_body.is_some());
        assert_eq!(synced.name.as_deref(), Some("Coin 7"));
        assert_eq!(synced.transform.unwrap().position, [3.0, 1.0, 0.0]);

        // Reverting one override takes the prefab's value back.
        let reverted = copy_paths(&changed, &synced, &["rigid_body".to_string()]).unwrap();
        assert!(reverted.rigid_body.is_none());
        assert_eq!(
            overrides(&changed, &reverted),
            ["mesh_visual.color", "attributes.value"]
        );
    }

    #[test]
    fn removed_components_and_attributes_are_overrides_too() {
        let base = coin();
        let mut instance = base.clone();
        instance.mesh_visual = None;
        instance.attributes.clear();
        let ov = overrides(&base, &instance);
        assert_eq!(ov, ["mesh_visual", "attributes"]);
        let mut changed = base;
        changed.mesh_visual.as_mut().unwrap().color = [0.0; 4];
        let synced = apply(&changed, &instance, &ov).unwrap();
        assert!(synced.mesh_visual.is_none());
        assert!(synced.attributes.is_empty());
    }

    #[test]
    fn scenes_resolve_instances_and_variants_from_disk() {
        let dir = std::env::temp_dir().join(format!("automata-prefab-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("prefabs")).unwrap();
        let write = |name: &str, entity: EntityData| {
            let file = SceneFile {
                version: crate::SCENE_FILE_VERSION,
                entities: vec![entity],
            };
            std::fs::write(dir.join("prefabs").join(name), file.to_ron().unwrap()).unwrap();
        };
        write("coin.prefab", coin());
        // A variant: a big coin worth 10.
        let mut big = coin();
        big.name = Some("Big Coin".into());
        big.mesh_visual.as_mut().unwrap().primitive = PrimitiveData::Sphere { size: 1.2 };
        big.attributes
            .insert("value".into(), AttributeValue::Int(10));
        big.prefab = Some(PrefabLink {
            path: "prefabs/coin.prefab".into(),
            index: 0,
            overrides: overrides(&coin(), &big),
        });
        write("big_coin.prefab", big);

        let stale = |link: &str, at: f32| {
            let mut e = coin();
            e.name = Some(format!("{link} at {at}"));
            e.transform.as_mut().unwrap().position = [at, 0.0, 0.0];
            e.prefab = Some(PrefabLink::new(link));
            e
        };
        let mut scene = SceneFile {
            version: crate::SCENE_FILE_VERSION,
            entities: vec![
                stale("prefabs/coin.prefab", 1.0),
                stale("prefabs/big_coin.prefab", 2.0),
                stale("prefabs/missing.prefab", 3.0),
            ],
        };
        // The base turns blue after the scene was saved.
        let mut blue = coin();
        blue.mesh_visual.as_mut().unwrap().color = [0.0, 0.3, 1.0, 1.0];
        write("coin.prefab", blue);

        let problems = scene.resolve_prefabs(&dir);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("missing.prefab"), "{problems:?}");
        let [plain, big, missing] = &scene.entities[..] else {
            panic!()
        };
        let color = |e: &EntityData| e.mesh_visual.as_ref().unwrap().color;
        assert_eq!(color(plain), [0.0, 0.3, 1.0, 1.0]);
        assert_eq!(
            color(big),
            [0.0, 0.3, 1.0, 1.0],
            "the variant follows its base"
        );
        assert_eq!(
            big.mesh_visual.as_ref().unwrap().primitive,
            PrimitiveData::Sphere { size: 1.2 }
        );
        assert_eq!(big.attributes["value"], AttributeValue::Int(10));
        assert_eq!(big.transform.unwrap().position, [2.0, 0.0, 0.0]);
        assert_eq!(big.name.as_deref(), Some("prefabs/big_coin.prefab at 2"));
        assert_eq!(color(missing), [1.0, 0.8, 0.1, 1.0], "kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn override_paths_read_as_labels() {
        assert_eq!(describe("mesh_visual.color"), "Mesh Renderer › Color");
        assert_eq!(describe("attributes.max_hp"), "Attributes › max_hp");
        assert_eq!(describe("rigid_body"), "Rigidbody");
        assert_eq!(describe("transform.scale"), "Transform › Scale");
    }
}
