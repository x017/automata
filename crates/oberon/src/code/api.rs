//! Built-in knowledge of the behaviour-script API.
//!
//! Scripts are plain modules with `start(entity, ctx)` / `update(entity,
//! ctx)` hooks: nothing is imported and nothing is annotated, so a language
//! server can't know what `ctx` or `entity` are. This module can: it
//! resolves expressions such as `ctx.scene["Player"].transform.` from the
//! hook parameters (and simple local assignments) and offers the right
//! members, plus key names inside `ctx.input.pressed("…")`, entity names
//! inside `ctx.scene["…"]` and attribute names inside
//! `entity.attributes["…"]`. It also answers hover for API members.

use super::{Buffer, CompletionItem, Pos};

/// An API type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    /// `ctx`.
    Context,
    /// `ctx.input`.
    Input,
    /// `ctx.time`.
    Time,
    /// `ctx.physics`.
    Physics,
    /// `ctx.scene`.
    Scene,
    /// A scene entity.
    Entity,
    /// `entity.transform`.
    Transform,
    /// `entity.attributes`.
    Attributes,
    /// `entity.state` (a plain dict).
    Dict,
    /// `entity.camera` / `ctx.camera`.
    Camera,
    /// `ctx.ui`.
    Ui,
}

/// What a string argument names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgKind {
    Key,
    EntityName,
    Attribute,
}

/// One member of an API type.
#[derive(Debug, Clone, Copy)]
pub struct Member {
    /// Member name.
    pub name: &'static str,
    /// Signature or type, e.g. `(key) -> bool`.
    pub sig: &'static str,
    /// One-line documentation.
    pub doc: &'static str,
    /// Whether it's a method.
    pub method: bool,
    /// The type it evaluates to (for properties) or returns (methods).
    pub ty: Option<Ty>,
    /// What a string argument names (key, entity, attribute).
    arg: Option<ArgKind>,
}

const fn prop(name: &'static str, sig: &'static str, doc: &'static str, ty: Option<Ty>) -> Member {
    Member {
        name,
        sig,
        doc,
        method: false,
        ty,
        arg: None,
    }
}

const fn meth(name: &'static str, sig: &'static str, doc: &'static str, ty: Option<Ty>) -> Member {
    Member {
        name,
        sig,
        doc,
        method: true,
        ty,
        arg: None,
    }
}

const fn with_arg(mut m: Member, arg: ArgKind) -> Member {
    m.arg = Some(arg);
    m
}

const CONTEXT: &[Member] = &[
    prop(
        "input",
        "Input",
        "Keyboard state this frame.",
        Some(Ty::Input),
    ),
    prop(
        "time",
        "Time",
        "Frame timing: delta, elapsed, frame.",
        Some(Ty::Time),
    ),
    prop("physics", "Physics", "Physics queries.", Some(Ty::Physics)),
    prop(
        "scene",
        "Scene",
        "The running scene's entities: ctx.scene[\"Name\"].",
        Some(Ty::Scene),
    ),
    prop(
        "camera",
        "Camera | None",
        "The scene camera Play mode renders through (None without one).",
        Some(Ty::Camera),
    ),
    prop(
        "hud",
        "Hud",
        "Shapes over the game for one frame: text, rect, image (centred); size, center.",
        None,
    ),
    prop(
        "ui",
        "Ui",
        "Game UI for this frame: button, toggle, slider, progress, label, panel, image.",
        Some(Ty::Ui),
    ),
    prop(
        "save",
        "SaveData",
        "A dict kept between runs: ctx.save[\"coins\"] = 12. Slots: use(), slots().",
        None,
    ),
    prop(
        "settings",
        "Settings",
        "The player's settings, kept between runs: volume, graphics, bind(action, *keys).",
        None,
    ),
    meth(
        "log",
        "(*parts) -> None",
        "Writes an info line to Oberon's Console.",
        None,
    ),
    meth(
        "warn",
        "(*parts) -> None",
        "Writes a warning to Oberon's Console.",
        None,
    ),
];

const INPUT: &[Member] = &[
    with_arg(
        meth(
            "pressed",
            "(key: str) -> bool",
            "Whether the key is held, e.g. pressed(\"space\").",
            None,
        ),
        ArgKind::Key,
    ),
    with_arg(
        meth(
            "just_pressed",
            "(key: str) -> bool",
            "True only on the frame the key went down.",
            None,
        ),
        ArgKind::Key,
    ),
    with_arg(
        meth(
            "just_released",
            "(key: str) -> bool",
            "True only on the frame the key went up.",
            None,
        ),
        ArgKind::Key,
    ),
    with_arg(
        meth(
            "axis",
            "(negative: str, positive: str) -> float",
            "-1, 0 or 1 from two keys, e.g. axis(\"a\", \"d\").",
            None,
        ),
        ArgKind::Key,
    ),
    meth(
        "any_just_pressed",
        "() -> str | None",
        "The key that went down this frame (for rebinding screens), or None.",
        None,
    ),
    meth(
        "mouse_pressed",
        "(button: str = \"left\") -> bool",
        "Whether a mouse button is held: \"left\", \"right\" or \"middle\".",
        None,
    ),
    meth(
        "mouse_just_pressed",
        "(button: str = \"left\") -> bool",
        "True only on the frame the button went down (not on ctx.ui).",
        None,
    ),
    meth(
        "mouse_just_released",
        "(button: str = \"left\") -> bool",
        "True only on the frame the button went up.",
        None,
    ),
    prop(
        "mouse_delta",
        "(dx, dy)",
        "Mouse movement in pixels since last frame; +y is down.",
        None,
    ),
    prop(
        "mouse_position",
        "(x, y) | None",
        "The cursor in screen pixels from the top-left; None outside the view or captured.",
        None,
    ),
    prop(
        "mouse_over_ui",
        "bool",
        "Whether the cursor is over ctx.ui (whose clicks the game doesn't see).",
        None,
    ),
    meth(
        "lock_mouse",
        "(locked: bool = True) -> None",
        "Hides and captures the cursor for mouse look; False frees it.",
        None,
    ),
    prop(
        "mouse_locked",
        "bool",
        "Whether the cursor is captured.",
        None,
    ),
];

const UI: &[Member] = &[
    meth(
        "button",
        "(text, x, y, width=160, height=44, *, id=None, enabled=True, size=None) -> bool",
        "A button at top-left (x, y): True on the frame it's clicked.",
        None,
    ),
    meth(
        "toggle",
        "(text, value, x, y, *, id=None, enabled=True, size=None) -> bool",
        "A check box with a label: returns the value, flipped when clicked.",
        None,
    ),
    meth(
        "slider",
        "(value, x, y, width=200, height=24, *, min=0, max=1, step=None, id=None, enabled=True) -> float",
        "A horizontal slider: returns the value after dragging.",
        None,
    ),
    meth(
        "progress",
        "(value, x, y, width=200, height=18, *, max=1, color=None, text=None) -> None",
        "A bar filled to value / max; text=True shows the percentage.",
        None,
    ),
    meth(
        "label",
        "(text, x, y, size=None, color=None, align=\"left\", shadow=None) -> None",
        "Text with its top at y.",
        None,
    ),
    meth(
        "panel",
        "(x, y, width, height, *, color=None, title=None) -> None",
        "A background box; clicks on it belong to the UI.",
        None,
    ),
    meth(
        "image",
        "(path, x, y, width, height, color=(1, 1, 1, 1)) -> None",
        "An image stretched over the box.",
        None,
    ),
    prop(
        "theme",
        "UiTheme",
        "Colours and text size: text, panel, button, hover, press, accent, track, border, text_size.",
        None,
    ),
    prop(
        "mouse_over",
        "bool",
        "Whether the cursor is over a widget or panel.",
        None,
    ),
];

const TIME: &[Member] = &[
    prop("delta", "float", "Seconds since the previous frame.", None),
    prop("elapsed", "float", "Seconds since Play started.", None),
    prop("frame", "int", "Frame number (starts at 1).", None),
];

const PHYSICS: &[Member] = &[meth(
    "grounded",
    "(entity) -> bool",
    "Whether a character-controlled entity stands on something.",
    None,
)];

const SCENE: &[Member] = &[
    with_arg(
        meth(
            "find",
            "(name: str) -> Entity | None",
            "The entity with this name, or None.",
            Some(Ty::Entity),
        ),
        ArgKind::EntityName,
    ),
    meth("names", "() -> list[str]", "Every entity's name.", None),
    prop(
        "entity_count",
        "int",
        "Number of entities in the scene.",
        None,
    ),
];

const ENTITY: &[Member] = &[
    prop("name", "str", "The entity's name.", None),
    prop("id", "int", "Stable id for this Play session.", None),
    prop(
        "transform",
        "Transform",
        "Position, rotation and scale.",
        Some(Ty::Transform),
    ),
    prop(
        "attributes",
        "Attributes",
        "Custom values from the Inspector: entity.attributes[\"health\"].",
        Some(Ty::Attributes),
    ),
    prop(
        "state",
        "dict",
        "Per-entity storage that persists between frames.",
        Some(Ty::Dict),
    ),
    prop(
        "character_velocity",
        "(x, y, z) | None",
        "Set to move a Character Controller (units per second).",
        None,
    ),
    prop(
        "grounded",
        "bool",
        "Whether the character stands on something.",
        None,
    ),
    prop("script", "str | None", "Path of the attached script.", None),
    prop(
        "camera",
        "Camera | None",
        "This entity's Camera component, or None.",
        Some(Ty::Camera),
    ),
    prop(
        "joint",
        "Joint | None",
        "This entity's Joint while it runs: angle, value, force, broken; motor_target, enabled.",
        None,
    ),
];

const CAMERA: &[Member] = &[
    prop(
        "position",
        "(x, y, z)",
        "The eye position; assign a tuple to move the camera.",
        None,
    ),
    prop(
        "target",
        "(x, y, z)",
        "The point the camera looks at.",
        None,
    ),
    prop("fov", "float", "Vertical field of view in degrees.", None),
    meth(
        "look_at",
        "(point_or_entity) -> None",
        "Aims the camera at a point or an entity.",
        None,
    ),
    meth(
        "follow",
        "(entity, offset=(0, 3, 8), smooth=8.0, look_height=0.6) -> None",
        "Eases toward entity + offset and looks at it; call every update.",
        None,
    ),
];

const TRANSFORM: &[Member] = &[
    prop(
        "position",
        "(x, y, z)",
        "Local position; assign a tuple to move.",
        None,
    ),
    prop(
        "rotation_euler",
        "(x, y, z)",
        "Euler rotation in radians.",
        None,
    ),
    prop("scale", "(x, y, z)", "Local scale.", None),
    meth(
        "translate",
        "(delta: (x, y, z)) -> None",
        "Moves by delta.",
        None,
    ),
    meth(
        "rotate",
        "(delta: (x, y, z)) -> None",
        "Rotates by delta radians (Euler).",
        None,
    ),
];

const ATTRIBUTES: &[Member] = &[
    with_arg(
        meth(
            "get",
            "(name: str, default=None)",
            "The attribute's value, or default.",
            None,
        ),
        ArgKind::Attribute,
    ),
    meth("keys", "() -> keys", "Attribute names.", None),
    meth("values", "() -> values", "Attribute values.", None),
    meth("items", "() -> items", "(name, value) pairs.", None),
    with_arg(
        meth(
            "pop",
            "(name: str, default=...)",
            "Removes an attribute and returns its value.",
            None,
        ),
        ArgKind::Attribute,
    ),
    with_arg(
        meth(
            "setdefault",
            "(name: str, default=None)",
            "Gets an attribute, creating it if missing.",
            None,
        ),
        ArgKind::Attribute,
    ),
    meth(
        "update",
        "(other) -> None",
        "Sets several attributes.",
        None,
    ),
];

const DICT: &[Member] = &[
    meth(
        "get",
        "(key, default=None)",
        "The value for key, or default.",
        None,
    ),
    meth("keys", "() -> keys", "The keys.", None),
    meth("values", "() -> values", "The values.", None),
    meth("items", "() -> items", "(key, value) pairs.", None),
    meth(
        "pop",
        "(key, default=...)",
        "Removes a key and returns its value.",
        None,
    ),
    meth(
        "setdefault",
        "(key, default=None)",
        "Gets a key, inserting default if missing.",
        None,
    ),
    meth(
        "update",
        "(other) -> None",
        "Merges other into the dict.",
        None,
    ),
];

/// Key names `ctx.input` understands, most common first.
pub const KEY_NAMES: &[&str] = &[
    "w",
    "a",
    "s",
    "d",
    "space",
    "shift",
    "ctrl",
    "alt",
    "arrow_left",
    "arrow_right",
    "arrow_up",
    "arrow_down",
    "enter",
    "escape",
    "tab",
    "backspace",
    "q",
    "e",
    "r",
    "f",
    "b",
    "c",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "t",
    "u",
    "v",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "shift_left",
    "shift_right",
    "ctrl_left",
    "ctrl_right",
    "alt_left",
    "alt_right",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

impl Ty {
    /// The type's members.
    #[must_use]
    pub fn members(self) -> &'static [Member] {
        match self {
            Self::Context => CONTEXT,
            Self::Input => INPUT,
            Self::Time => TIME,
            Self::Physics => PHYSICS,
            Self::Scene => SCENE,
            Self::Entity => ENTITY,
            Self::Transform => TRANSFORM,
            Self::Attributes => ATTRIBUTES,
            Self::Dict => DICT,
            Self::Camera => CAMERA,
            Self::Ui => UI,
        }
    }

    /// The type's display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Context => "Context",
            Self::Input => "Input",
            Self::Time => "Time",
            Self::Physics => "Physics",
            Self::Scene => "Scene",
            Self::Entity => "Entity",
            Self::Transform => "Transform",
            Self::Attributes => "Attributes",
            Self::Dict => "dict",
            Self::Camera => "Camera",
            Self::Ui => "Ui",
        }
    }

    fn doc(self) -> &'static str {
        match self {
            Self::Context => {
                "The per-frame context passed to start(entity, ctx) / update(entity, ctx)."
            }
            Self::Entity => "The entity this script is attached to (or another scene entity).",
            Self::Input => "Keyboard state this frame.",
            Self::Time => "Frame timing.",
            Self::Physics => "Physics queries.",
            Self::Scene => "The running scene's entities.",
            Self::Transform => "An entity's position, rotation and scale.",
            Self::Attributes => "Custom values set in the Inspector.",
            Self::Dict => "Per-entity storage that persists between frames.",
            Self::Camera => "A perspective camera: position, target, fov.",
            Self::Ui => "Game UI widgets, drawn by calling them every frame.",
        }
    }

    fn index(self) -> Option<Ty> {
        match self {
            Self::Scene => Some(Self::Entity),
            _ => None,
        }
    }

    fn index_arg(self) -> Option<ArgKind> {
        match self {
            Self::Scene => Some(ArgKind::EntityName),
            Self::Attributes => Some(ArgKind::Attribute),
            _ => None,
        }
    }

    fn iter_item(self) -> Option<Ty> {
        match self {
            Self::Scene => Some(Self::Entity),
            _ => None,
        }
    }
}

/// Names from the open scene for string-argument completion.
#[derive(Debug, Clone, Default)]
pub struct SceneNames {
    /// Entity names.
    pub entities: Vec<String>,
    /// Custom attribute names (all entities).
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Seg {
    Name(String),
    Attr(String),
    Index,
    Call,
}

/// Completions for the caret at `pos`, or `None` when the expression isn't
/// part of the script API (the language server handles it).
#[must_use]
pub fn complete(buf: &Buffer, pos: Pos, names: &SceneNames) -> Option<Vec<CompletionItem>> {
    let line = buf.line(pos.line);
    let before: String = line.chars().take(pos.col).collect();

    // Inside a string argument: key / entity / attribute names.
    let (quote, comment) = scan_line(&before);
    if comment {
        return None;
    }
    if let Some(quote_at) = quote {
        let head = before[..quote_at].trim_end();
        let kind = string_arg_kind(buf, pos.line, head)?;
        let list: Vec<String> = match kind {
            ArgKind::Key => KEY_NAMES.iter().map(|k| (*k).to_string()).collect(),
            ArgKind::EntityName => names.entities.clone(),
            ArgKind::Attribute => names.attributes.clone(),
        };
        let (kind_char, detail) = match kind {
            ArgKind::Key => (super::CompletionKind::Key, "key"),
            ArgKind::EntityName => (super::CompletionKind::Entity, "entity"),
            ArgKind::Attribute => (super::CompletionKind::Property, "attribute"),
        };
        return Some(
            list.into_iter()
                .map(|n| CompletionItem {
                    label: n.clone(),
                    detail: detail.to_string(),
                    insert: n,
                    kind: kind_char,
                })
                .collect(),
        );
    }

    // `expr.partial`
    let trimmed = before.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    let receiver = trimmed.strip_suffix('.')?;
    let chain = parse_chain(receiver)?;
    let ty = resolve(buf, pos.line, &chain, 0)?;
    Some(
        ty.members()
            .iter()
            .map(|m| CompletionItem {
                label: m.name.to_string(),
                detail: m.sig.to_string(),
                insert: m.name.to_string(),
                kind: if m.method {
                    super::CompletionKind::Method
                } else {
                    super::CompletionKind::Property
                },
            })
            .collect(),
    )
}

/// Hover text for the API name under `pos`.
#[must_use]
pub fn hover(buf: &Buffer, pos: Pos) -> Option<String> {
    let line = buf.line(pos.line);
    let chars: Vec<char> = line.chars().collect();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    if !chars.get(pos.col).copied().is_some_and(is_ident) {
        return None;
    }
    let mut end = pos.col;
    while end < chars.len() && is_ident(chars[end]) {
        end += 1;
    }
    let upto: String = chars[..end].iter().collect();
    let chain = parse_chain(&upto)?;
    if chain.len() == 1 {
        let ty = resolve(buf, pos.line, &chain, 0)?;
        let Seg::Name(n) = &chain[0] else { return None };
        return Some(format!("{n}: {}\n{}", ty.name(), ty.doc()));
    }
    let (last, head) = chain.split_last()?;
    let Seg::Attr(name) = last else { return None };
    let ty = resolve(buf, pos.line, head, 0)?;
    let m = ty.members().iter().find(|m| m.name == name)?;
    let sep = if m.method { "" } else { ": " };
    Some(format!("{}.{}{sep}{}\n{}", ty.name(), m.name, m.sig, m.doc))
}

/// Where `before` ends: inside a string (the opening quote's byte index)
/// and / or inside a comment.
fn scan_line(before: &str) -> (Option<usize>, bool) {
    let mut open: Option<(usize, char)> = None;
    let mut escaped = false;
    for (i, c) in before.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (open, c) {
            (Some(_), '\\') => escaped = true,
            (Some((_, q)), c) if c == q => open = None,
            (None, '"' | '\'') => open = Some((i, c)),
            (None, '#') => return (None, true),
            _ => {}
        }
    }
    (open.map(|(i, _)| i), false)
}

/// What the string being typed after `head` names (`head` ends with the
/// opening `(` / `[`, possibly followed by earlier arguments).
fn string_arg_kind(buf: &Buffer, line: usize, head: &str) -> Option<ArgKind> {
    // Find the unmatched opener.
    let bytes = head.as_bytes();
    let mut depth = 0i32;
    let mut at = None;
    for i in (0..bytes.len()).rev() {
        match bytes[i] {
            b')' | b']' => depth += 1,
            b'(' | b'[' if depth > 0 => depth -= 1,
            b'(' | b'[' => {
                at = Some(i);
                break;
            }
            _ => {}
        }
    }
    let at = at?;
    let opener = bytes[at];
    let chain = parse_chain(head[..at].trim_end())?;
    if opener == b'[' {
        return resolve(buf, line, &chain, 0)?.index_arg();
    }
    let (last, recv) = chain.split_last()?;
    let Seg::Attr(name) = last else { return None };
    let ty = resolve(buf, line, recv, 0)?;
    ty.members().iter().find(|m| m.name == name)?.arg
}

/// Parses the dotted expression at the end of `text` (identifiers, `.attr`,
/// `[...]`, `(...)`), e.g. `x = ctx.scene["a"].transform` →
/// `[ctx, scene, [], transform]`.
fn parse_chain(text: &str) -> Option<Vec<Seg>> {
    let chars: Vec<char> = text.chars().collect();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = chars.len();
    let mut segs = Vec::new();
    loop {
        // A trailing call / index.
        while i > 0 && matches!(chars[i - 1], ')' | ']') {
            let close = chars[i - 1];
            let open = if close == ')' { '(' } else { '[' };
            let mut depth = 0;
            let mut j = i;
            loop {
                if j == 0 {
                    return None;
                }
                j -= 1;
                if chars[j] == close {
                    depth += 1;
                } else if chars[j] == open {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            segs.push(if close == ')' { Seg::Call } else { Seg::Index });
            i = j;
        }
        let end = i;
        while i > 0 && is_ident(chars[i - 1]) {
            i -= 1;
        }
        if i == end {
            return None;
        }
        let name: String = chars[i..end].iter().collect();
        if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        if i > 0 && chars[i - 1] == '.' {
            segs.push(Seg::Attr(name));
            i -= 1;
        } else {
            segs.push(Seg::Name(name));
            break;
        }
    }
    segs.reverse();
    Some(segs)
}

/// The type of `chain` at `line` (hook parameters, then local assignments).
fn resolve(buf: &Buffer, line: usize, chain: &[Seg], depth: u8) -> Option<Ty> {
    let (Seg::Name(root), rest) = chain.split_first()? else {
        return None;
    };
    let mut ty = root_type(buf, line, root, depth)?;
    for seg in rest {
        ty = match seg {
            Seg::Attr(name) => ty.members().iter().find(|m| m.name == name)?.ty?,
            Seg::Index => ty.index()?,
            // A method's `ty` is already its return type.
            Seg::Call => ty,
            Seg::Name(_) => return None,
        };
    }
    Some(ty)
}

/// The type of a bare name at `line`.
fn root_type(buf: &Buffer, line: usize, name: &str, depth: u8) -> Option<Ty> {
    if depth > 4 {
        return None;
    }
    let (def_line, params, fn_name) = enclosing_def(buf, line)?;
    // Local assignments / loops between the def and `line`, last one wins.
    for l in (def_line + 1..=line).rev() {
        let text = buf.line(l);
        let t = text.trim();
        if let Some(rest) = t.strip_prefix("for ") {
            if let Some((var, iter)) = rest.split_once(" in ") {
                if var.trim() == name {
                    let iter = iter.trim_end_matches(':').trim();
                    let chain = parse_chain(iter)?;
                    return resolve(buf, l, &chain, depth + 1)?.iter_item();
                }
            }
        }
        if l == line {
            continue;
        }
        if let Some((lhs, rhs)) = t.split_once('=') {
            let lhs = lhs.trim();
            let op = rhs.starts_with('=') || lhs.ends_with(['!', '<', '>', '+', '-', '*', '/']);
            if lhs == name && !op {
                let rhs = rhs.split('#').next().unwrap_or("").trim();
                let rhs = rhs.strip_suffix(" or None").unwrap_or(rhs);
                return resolve(buf, l, &parse_chain(rhs)?, depth + 1);
            }
        }
    }
    let idx = params.iter().position(|p| p == name)?;
    let hook = matches!(
        fn_name.as_str(),
        "start"
            | "update"
            | "on_start"
            | "on_update"
            | "stop"
            | "on_stop"
            | "fixed_update"
            | "late_update"
    );
    match name {
        "ctx" | "context" => return Some(Ty::Context),
        "entity" => return Some(Ty::Entity),
        _ => {}
    }
    if (hook || params.get(1).is_some_and(|p| p == "ctx")) && params.len() >= 2 {
        return match idx {
            0 => Some(Ty::Entity),
            1 => Some(Ty::Context),
            _ => None,
        };
    }
    None
}

/// The nearest enclosing `def` above `line`: its line, parameter names and
/// function name.
fn enclosing_def(buf: &Buffer, line: usize) -> Option<(usize, Vec<String>, String)> {
    let cur_indent = indent_width(buf, line);
    for l in (0..=line).rev() {
        let text = buf.line(l);
        let t = text.trim_start();
        let indent = text.len() - t.len();
        let Some(rest) = t
            .strip_prefix("def ")
            .or_else(|| t.strip_prefix("async def "))
        else {
            continue;
        };
        if l != line && indent >= cur_indent && !text.trim().is_empty() {
            continue;
        }
        let (fname, rest) = rest.split_once('(')?;
        let params_text = rest.split(')').next().unwrap_or("");
        let params = params_text
            .split(',')
            .map(|p| {
                p.split([':', '='])
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_start_matches('*')
                    .to_string()
            })
            .filter(|p| !p.is_empty() && p != "self")
            .collect();
        return Some((l, params, fname.trim().to_string()));
    }
    None
}

/// Indentation of `line`, or of the next non-blank line when it's blank.
fn indent_width(buf: &Buffer, line: usize) -> usize {
    (line..buf.line_count())
        .map(|l| buf.line(l))
        .find(|t| !t.trim().is_empty())
        .map_or(usize::MAX, |t| t.len() - t.trim_start().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "SPEED = 5.0\n\ndef update(entity, ctx):\n    player = ctx.scene[\"Player\"]\n    hero = ctx.scene.find(\"Hero\")\n    for e in ctx.scene:\n        e.\n    x = 1\n";

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    fn at(src: &str, line: usize, extra: &str) -> (Buffer, Pos) {
        let mut text: Vec<String> = src.lines().map(str::to_string).collect();
        text[line].push_str(extra);
        let col = text[line].chars().count();
        (Buffer::from_text(&text.join("\n")), Pos::new(line, col))
    }

    #[test]
    fn completes_ctx_members_without_imports() {
        let names = SceneNames::default();
        let (b, p) = at(SCRIPT, 7, "  # ctx.");
        assert!(complete(&b, p, &names).is_none(), "comments aren't code");
        for (expr, expect) in [
            ("ctx.", "input"),
            ("ctx.input.", "pressed"),
            ("ctx.time.", "delta"),
            ("entity.", "transform"),
            ("entity.transform.", "position"),
            ("entity.attributes.", "get"),
            ("player.", "character_velocity"),
            ("hero.", "name"),
            ("ctx.scene[\"x\"].transform.", "rotation_euler"),
            ("ctx.scene.fi", "find"),
            ("ctx.ui.", "button"),
            ("ctx.ui.sl", "slider"),
            ("ctx.input.", "mouse_over_ui"),
            ("entity.", "joint"),
        ] {
            let (b, p) = at(SCRIPT, 7, &format!("; {expr}"));
            let items = complete(&b, p, &names).unwrap_or_else(|| panic!("{expr}: no items"));
            assert!(
                labels(&items).contains(&expect),
                "{expr}: {:?}",
                labels(&items)
            );
        }
        // A loop variable over the scene is an Entity.
        let b = Buffer::from_text(SCRIPT);
        let items = complete(&b, Pos::new(6, 10), &names).unwrap();
        assert!(labels(&items).contains(&"transform"));
        // Unknown names fall through to the language server.
        let (b, p) = at(SCRIPT, 7, "; SPEED.");
        assert!(complete(&b, p, &names).is_none());
    }

    #[test]
    fn completes_string_arguments() {
        let names = SceneNames {
            entities: vec!["Player".into(), "Crate 1".into()],
            attributes: vec!["health".into()],
        };
        let (b, p) = at(SCRIPT, 7, "; ctx.input.pressed(\"sp");
        assert!(labels(&complete(&b, p, &names).unwrap()).contains(&"space"));
        let (b, p) = at(SCRIPT, 7, "; ctx.input.axis(\"a\", \"");
        assert!(labels(&complete(&b, p, &names).unwrap()).contains(&"d"));
        let (b, p) = at(SCRIPT, 7, "; ctx.scene[\"");
        assert_eq!(
            labels(&complete(&b, p, &names).unwrap()),
            vec!["Player", "Crate 1"]
        );
        let (b, p) = at(SCRIPT, 7, "; entity.attributes[\"h");
        assert_eq!(labels(&complete(&b, p, &names).unwrap()), vec!["health"]);
        let (b, p) = at(SCRIPT, 7, "; entity.attributes.get('");
        assert_eq!(labels(&complete(&b, p, &names).unwrap()), vec!["health"]);
        let (b, p) = at(SCRIPT, 7, "; print(\"he");
        assert!(complete(&b, p, &names).is_none());
    }

    #[test]
    fn hovers_api_members() {
        let b = Buffer::from_text(
            "def update(entity, ctx):\n    if ctx.input.pressed(\"w\"):\n        pass\n",
        );
        let h = hover(&b, Pos::new(1, 18)).unwrap();
        assert!(h.starts_with("Input.pressed(key: str) -> bool"), "{h}");
        let h = hover(&b, Pos::new(1, 8)).unwrap();
        assert!(h.starts_with("ctx: Context"), "{h}");
        assert!(hover(&b, Pos::new(2, 9)).is_none());
    }
}
