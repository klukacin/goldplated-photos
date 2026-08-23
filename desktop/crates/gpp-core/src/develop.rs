//! Non-destructive develop: adjustments recorded, never baked into originals.
//!
//! An edit is a list of operations stored as JSON against a photo. Nothing on
//! disk changes when you move a slider — the original file is never opened for
//! writing. Adjusted pixels appear in two places, both derived and both
//! rebuildable: the thumbnail cache, and whatever gets published.
//!
//! # How it stays fast
//!
//! Every derived image is addressed by a **render key**: the content hash for
//! an untouched photo, or a hash of the content plus the edit stack for an
//! adjusted one. That single idea buys three things:
//!
//! - a photo with no edits keeps its existing thumbnails, so adding this
//!   feature invalidates nothing;
//! - changing an edit changes the key, so stale thumbnails can never be shown;
//! - reverting to a previous edit returns to a key that is probably still
//!   cached, so undo is instant.
//!
//! Publishing copies the cached full-size render rather than re-developing, so
//! an album of adjusted photos costs one render each, not one per publish.

use std::path::{Path, PathBuf};

use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::media;
use crate::model::Photo;

/// Current stack format. Bumped only if the meaning of stored ops changes.
pub const STACK_VERSION: u32 = 1;

/// JPEG quality for a developed frame — the file a client is actually sent.
///
/// An untouched photo is published by copying the camera's own file, so it
/// arrives at whatever quality the body wrote, typically the low nineties. An
/// adjusted one is re-encoded here, and it has to land in the same range or
/// moving one slider silently costs the delivered frame detail that was
/// recorded at the wedding. Grid thumbnails are a separate decision — see
/// `media::THUMBNAIL_JPEG_QUALITY`, which is lower on purpose because nobody is
/// ever sent one.
pub const DELIVERY_JPEG_QUALITY: u8 = 92;

/// One adjustment.
///
/// Amounts are the -100..100 scale a slider hands over, except exposure, which
/// is in stops because that is the unit photographers think in. Values are
/// clamped by [`clamped`](Self::clamped) on the way into a stack, but a stack
/// read back from the catalog has not been through that — treat anything
/// arriving from disk as unbounded, including NaN.
///
/// The serialized field names are the on-disk format (`{"op":"exposure",...}`)
/// and they feed the render key, so renaming one orphans every cached render in
/// every library that has the old spelling.
///
/// The four geometry variants are described individually below, but the rule
/// that governs them is not in any one of them: see [`EditStack`] — turns and
/// mirrors are folded into a canonical framing rather than edited where they
/// lie, and nothing may infer orientation from a single op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum EditOp {
    /// Stops. +1 doubles the light, -1 halves it.
    Exposure { ev: f32 },
    /// Pivoted on mid grey, so pushing contrast does not also brighten or
    /// darken the frame overall.
    Contrast { amount: f32 },
    /// Scales each channel's distance from the pixel's own luma, so -100 lands
    /// on neutral grey rather than on black.
    Saturation { amount: f32 },
    /// Positive is warmer.
    Temperature { amount: f32 },
    /// Positive is magenta, negative green.
    Tint { amount: f32 },
    /// Recover blown highlights (negative) or lift them (positive).
    ///
    /// Masked by how bright the pixel already is, so a shadow is left exactly
    /// where it was — a recovery slider that also lifted the blacks would be
    /// unusable on a backlit ceremony.
    Highlights { amount: f32 },
    /// The other end of the same mask: weighted towards the dark pixels, and
    /// nothing above mid grey moves.
    Shadows { amount: f32 },
    /// Drop colour, keeping luminance.
    BlackAndWhite,
    /// Quarter turns clockwise, 0..3.
    ///
    /// At most one of these is ever stored, and its value is absolute, not a
    /// press: the buttons go through [`EditStack::rotate_by`], which composes.
    Rotate { quarter_turns: u8 },
    /// A left-to-right mirror — the only mirror the canonical form uses.
    FlipHorizontal,
    /// A top-to-bottom mirror.
    ///
    /// Accepted from callers and from stacks written by older versions, but
    /// never written by [`EditStack`]: a vertical mirror is a horizontal one
    /// plus a half turn, and storing it that way is what keeps the eight
    /// orientations closed under the buttons. Code looking for a vertical flip
    /// in a stack will not find one, and should not be looking.
    FlipVertical,
    /// Fractions of the frame, each 0..1, taken after the turns and mirrors.
    ///
    /// The rectangle only means what the photographer drew while the framing
    /// beneath it stays put, which is why the crop is kept last in the stack
    /// and carried through every subsequent turn.
    Crop { x: f32, y: f32, w: f32, h: f32 },
}

impl EditOp {
    /// A stable name for the *kind* of adjustment, ignoring its value.
    ///
    /// Used to upsert: moving the exposure slider twice must replace the first
    /// value, not stack a second exposure op on top of it.
    pub fn kind(&self) -> &'static str {
        match self {
            EditOp::Exposure { .. } => "exposure",
            EditOp::Contrast { .. } => "contrast",
            EditOp::Saturation { .. } => "saturation",
            EditOp::Temperature { .. } => "temperature",
            EditOp::Tint { .. } => "tint",
            EditOp::Highlights { .. } => "highlights",
            EditOp::Shadows { .. } => "shadows",
            EditOp::BlackAndWhite => "black-and-white",
            EditOp::Rotate { .. } => "rotate",
            EditOp::FlipHorizontal => "flip-horizontal",
            EditOp::FlipVertical => "flip-vertical",
            EditOp::Crop { .. } => "crop",
        }
    }

    /// True when the op would not change a single pixel, so it can be dropped
    /// rather than stored — which keeps an untouched photo on its original
    /// render key and its existing thumbnails.
    pub fn is_identity(&self) -> bool {
        match *self {
            EditOp::Exposure { ev } => ev == 0.0,
            EditOp::Contrast { amount }
            | EditOp::Saturation { amount }
            | EditOp::Temperature { amount }
            | EditOp::Tint { amount }
            | EditOp::Highlights { amount }
            | EditOp::Shadows { amount } => amount == 0.0,
            EditOp::Rotate { quarter_turns } => quarter_turns % 4 == 0,
            EditOp::Crop { x, y, w, h } => x == 0.0 && y == 0.0 && w == 1.0 && h == 1.0,
            EditOp::BlackAndWhite | EditOp::FlipHorizontal | EditOp::FlipVertical => false,
        }
    }

    /// Clamp to the ranges the renderer is defined over. A UI can send anything;
    /// the core decides what is meaningful.
    pub fn clamped(self) -> Self {
        let a = |v: f32| v.clamp(-100.0, 100.0);
        match self {
            EditOp::Exposure { ev } => EditOp::Exposure { ev: ev.clamp(-5.0, 5.0) },
            EditOp::Contrast { amount } => EditOp::Contrast { amount: a(amount) },
            EditOp::Saturation { amount } => EditOp::Saturation { amount: a(amount) },
            EditOp::Temperature { amount } => EditOp::Temperature { amount: a(amount) },
            EditOp::Tint { amount } => EditOp::Tint { amount: a(amount) },
            EditOp::Highlights { amount } => EditOp::Highlights { amount: a(amount) },
            EditOp::Shadows { amount } => EditOp::Shadows { amount: a(amount) },
            EditOp::Rotate { quarter_turns } => EditOp::Rotate {
                quarter_turns: quarter_turns % 4,
            },
            EditOp::Crop { x, y, w, h } => {
                let x = x.clamp(0.0, 1.0);
                let y = y.clamp(0.0, 1.0);
                EditOp::Crop {
                    x,
                    y,
                    w: w.clamp(0.0, 1.0 - x).max(f32::MIN_POSITIVE),
                    h: h.clamp(0.0, 1.0 - y).max(f32::MIN_POSITIVE),
                }
            }
            other => other,
        }
    }
}

/// Which way up the photograph is: a left-to-right mirror, then a number of
/// quarter turns clockwise.
///
/// Those eight states are every way a rectangle can be set down, so any run of
/// rotate and flip presses adds up to one of them. That matters because turns
/// and mirrors do not commute, and the ops therefore cannot be edited where
/// they happen to lie in the stack — pressing "rotate right" on a frame that
/// had been flipped turned the photograph left, and pressing a flip a second
/// time to undo it mirrored the wrong axis. The only way a button can be
/// trusted is to compose it onto the *outside* of the framing already there and
/// write the whole thing back, which is what the methods here are for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Framing {
    mirrored: bool,
    turns: i32,
}

impl Framing {
    const UPRIGHT: Self = Self { mirrored: false, turns: 0 };

    /// Turn the already-framed picture, i.e. `rotate ∘ self`.
    fn then_turn(self, quarter_turns: i32) -> Self {
        Self {
            turns: (self.turns + quarter_turns).rem_euclid(4),
            ..self
        }
    }

    /// Mirror the already-framed picture left to right.
    ///
    /// `flip ∘ rotate(θ)` is `rotate(-θ) ∘ flip`, so pushing a mirror to the
    /// inside reverses the turn it passes.
    fn then_mirror_h(self) -> Self {
        Self {
            mirrored: !self.mirrored,
            turns: (-self.turns).rem_euclid(4),
        }
    }

    /// Mirror it top to bottom. A vertical mirror is a horizontal one and a
    /// half turn, which is why only one mirror needs storing.
    fn then_mirror_v(self) -> Self {
        let m = self.then_mirror_h();
        m.then_turn(2)
    }
}

/// Fold a run of ops into the one framing they add up to. Anything that is not
/// a turn or a mirror leaves the frame where it is.
fn framing_of(ops: &[EditOp]) -> Framing {
    let mut f = Framing::UPRIGHT;
    for op in ops {
        f = match op {
            EditOp::Rotate { quarter_turns } => f.then_turn(i32::from(*quarter_turns)),
            EditOp::FlipHorizontal => f.then_mirror_h(),
            EditOp::FlipVertical => f.then_mirror_v(),
            _ => f,
        };
    }
    f
}

/// Everything done to one photo, in order.
///
/// This is the whole of what "developed" means for a photograph: there is no
/// other record, and the original file is never written. An empty stack is not
/// a special case to branch on — it is the ordinary state of most of a library,
/// and it is what lets [`ensure_rendered`] hand back the camera's own file with
/// nothing copied and nothing cached.
///
/// # Geometry does not live where it looks like it lives
///
/// A caller may set tone ops freely. Turns and mirrors are different: they do
/// not commute, so the stack stores the single canonical framing they add up to
/// — one left-to-right mirror, then quarter turns — and every geometry button
/// ([`rotate_by`](Self::rotate_by), [`toggle`](Self::toggle), and
/// [`set`](Self::set) when handed a geometry op) composes onto the outside of
/// that and rewrites it. Editing an op where it sits instead turned a mirrored
/// photograph the wrong way, and made a second press of a flip mirror the wrong
/// axis. Anything reading orientation back must fold the whole op list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EditStack {
    /// Format of the stored ops, not of the photograph. Bumped only when the
    /// *meaning* of an existing op changes, because every catalog on disk holds
    /// stacks written by older builds and they have to keep rendering the
    /// picture the photographer approved. An absent field reads as the current
    /// version.
    #[serde(default = "default_version")]
    pub version: u32,
    /// The adjustments, in the order they were recorded.
    ///
    /// Order is meaning within tone — exposure then contrast is not contrast
    /// then exposure — and within geometry. It is *not* meaning between the
    /// two: [`apply`] runs geometry in one pass and tone in another, so where a
    /// tone op sits relative to a crop cannot change a pixel.
    #[serde(default)]
    pub ops: Vec<EditOp>,
}

fn default_version() -> u32 {
    STACK_VERSION
}

impl EditStack {
    pub fn new() -> Self {
        Self {
            version: STACK_VERSION,
            ops: Vec::new(),
        }
    }

    /// True when this photo is still the camera's file.
    ///
    /// More than a length check, because three other things key off it: the
    /// render key is the plain content hash, [`ensure_rendered`] returns the
    /// original's own path, and [`Library::set_edits`] deletes the row rather
    /// than storing an empty one. Something that reports edits on an untouched
    /// photo therefore also costs it every thumbnail it already had.
    ///
    /// [`Library::set_edits`]: crate::catalog::Library::set_edits
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Add or replace an adjustment of the same kind.
    ///
    /// An identity value removes the op instead of storing a no-op, so a slider
    /// returned to zero leaves no trace and the photo goes back to its original
    /// render key.
    pub fn set(&mut self, op: EditOp) {
        let op = op.clamped();

        // Orientation has exactly one road in, whoever is driving. This setter
        // is the generic one the C ABI exposes, and a foreign client reaching
        // geometry through it used to edit the op where it lay — the model that
        // turned a mirrored photograph the wrong way. Sending "turn right,
        // mirror, turn right" that way ended at a half turn plus a mirror where
        // the buttons end at a mirror alone: a different picture, on the same
        // three instructions.
        match op {
            EditOp::Rotate { quarter_turns } => {
                return self.set_orientation(i32::from(quarter_turns));
            }
            // A mirror carries no value, so "set" can only mean "apply it".
            EditOp::FlipHorizontal | EditOp::FlipVertical => return self.toggle(op),
            _ => {}
        }

        if op.is_identity() {
            self.ops.retain(|existing| existing.kind() != op.kind());
            return;
        }
        // Replaced where it stands. Dropping it and appending would move the op
        // to the end, and the stack *is* the order the operations run in: a
        // second press of "rotate right" slid the turn past a crop that had
        // been drawn on the turned frame, and the framing jumped.
        match self.ops.iter_mut().find(|existing| existing.kind() == op.kind()) {
            Some(existing) => *existing = op,
            None => self.ops.push(op),
        }
    }

    /// Read the turns and mirrors in the stack as one transform.
    fn framing(&self) -> Framing {
        framing_of(&self.ops)
    }

    /// Write a framing back, replacing every turn and mirror in the stack.
    ///
    /// The crop is kept at the end. Its rectangle is fractions of the frame
    /// directly beneath it, so it only means what the photographer drew if
    /// nothing re-orients that frame afterwards.
    fn set_framing(&mut self, f: Framing) {
        let crop = self
            .ops
            .iter()
            .position(|op| op.kind() == "crop")
            .map(|at| self.ops.remove(at));
        self.ops.retain(|op| {
            !matches!(
                op,
                EditOp::Rotate { .. } | EditOp::FlipHorizontal | EditOp::FlipVertical
            )
        });
        if f.mirrored {
            self.ops.push(EditOp::FlipHorizontal);
        }
        if f.turns.rem_euclid(4) != 0 {
            self.ops.push(EditOp::Rotate {
                quarter_turns: f.turns.rem_euclid(4) as u8,
            });
        }
        if let Some(crop) = crop {
            self.ops.push(crop);
        }
    }

    /// Move the crop rectangle the same way the frame beneath it just moved, so
    /// it goes on framing the same part of the photograph.
    ///
    /// Without this, straightening a shot after framing a face re-reads the same
    /// four fractions against a frame whose axes have swapped, and the face is
    /// simply gone — with nothing on screen to explain it.
    fn carry_crop(&mut self, turn: i32, mirror_h: bool, mirror_v: bool) {
        let Some(EditOp::Crop { x, y, w, h }) =
            self.ops.iter_mut().find(|op| op.kind() == "crop")
        else {
            return;
        };
        if mirror_h {
            *x = 1.0 - (*x + *w);
        }
        if mirror_v {
            *y = 1.0 - (*y + *h);
        }
        // A quarter turn clockwise sends the top-left corner to the top-right
        // and swaps the sides.
        for _ in 0..turn.rem_euclid(4) {
            let (nx, ny, nw, nh) = (1.0 - (*y + *h), *x, *h, *w);
            (*x, *y, *w, *h) = (nx, ny, nw, nh);
        }
    }

    /// Put the photograph at an absolute number of quarter turns, keeping any
    /// mirror and carrying the crop through the difference.
    ///
    /// The relative form the buttons use is [`rotate_by`](Self::rotate_by);
    /// this is what an absolute `Rotate` op means when one arrives from a
    /// caller that tracks the angle itself.
    fn set_orientation(&mut self, turns: i32) {
        self.normalize_geometry();
        let current = self.framing();
        let delta = turns - current.turns;
        self.set_framing(Framing {
            turns: turns.rem_euclid(4),
            ..current
        });
        self.carry_crop(delta, false, false);
    }

    /// Put a stack into the shape the buttons below assume: one canonical
    /// framing, with the crop last.
    ///
    /// A stack written before orientation was stored canonically can hold the
    /// crop *ahead* of the turns — the photographer framed the shot and then
    /// straightened it, and each op was appended where it fell. Such stacks are
    /// in catalogs on disk, and they still render correctly; it is the next
    /// press of a geometry button that breaks them, because [`set_framing`]
    /// moves the crop to the end, where the same four fractions are read
    /// against a frame that has since turned. The bride ends up outside the
    /// picture with nothing on screen to explain it. So the rectangle is first
    /// carried through whatever framing used to follow it.
    ///
    /// [`set_framing`]: Self::set_framing
    fn normalize_geometry(&mut self) {
        let Some(at) = self.ops.iter().position(|op| op.kind() == "crop") else {
            return;
        };
        let after = framing_of(&self.ops[at + 1..]);
        if after != Framing::UPRIGHT {
            // A framing is a mirror and then turns, and that is the order
            // `carry_crop` applies them in.
            self.carry_crop(after.turns, after.mirrored, false);
        }
        let whole = self.framing();
        self.set_framing(whole);
    }

    /// Turn a further quarter on top of whatever turn is already recorded.
    ///
    /// Rotate buttons are relative — two clicks of "right" mean 180°.
    pub fn rotate_by(&mut self, quarter_turns: i32) {
        self.normalize_geometry();
        let f = self.framing().then_turn(quarter_turns);
        self.set_framing(f);
        self.carry_crop(quarter_turns, false, false);
    }

    /// Switch a mirror on, or off again.
    ///
    /// The flips are the only adjustments with nothing to set to zero, so
    /// `set` alone could never undo one: it drops the existing op and pushes an
    /// identical one straight back.
    pub fn toggle(&mut self, op: EditOp) {
        match op {
            EditOp::FlipHorizontal => {
                self.normalize_geometry();
                let f = self.framing().then_mirror_h();
                self.set_framing(f);
                self.carry_crop(0, true, false);
            }
            EditOp::FlipVertical => {
                self.normalize_geometry();
                let f = self.framing().then_mirror_v();
                self.set_framing(f);
                self.carry_crop(0, false, true);
            }
            other => {
                if self.get(other.kind()).is_some() {
                    self.remove(other.kind());
                } else {
                    self.set(other);
                }
            }
        }
    }

    /// Drop every op of one kind, named as [`EditOp::kind`] spells it.
    ///
    /// Fine for tone. Reaching for it to undo geometry is not: taking out
    /// `"rotate"` leaves any mirror standing, and the result is a framing the
    /// photographer never asked for. The buttons undo themselves —
    /// [`rotate_by`](Self::rotate_by) with the opposite sign, or a second
    /// [`toggle`](Self::toggle).
    pub fn remove(&mut self, kind: &str) {
        self.ops.retain(|op| op.kind() != kind);
    }

    /// The op of one kind, if the stack holds it. Kinds are the strings
    /// [`EditOp::kind`] returns; an unknown one is simply not found.
    pub fn get(&self, kind: &str) -> Option<&EditOp> {
        self.ops.iter().find(|op| op.kind() == kind)
    }

    /// Serialize to the exact text stored in `edits.stack_json` — and hashed
    /// into the render key.
    ///
    /// The key covers this string, not the ops it describes, so anything that
    /// changes the *spelling* changes every key: a different field order, a
    /// float printed as `1` rather than `1.0`, pretty-printing. Nothing renders
    /// wrong, but every cached render and thumbnail in every existing library
    /// is orphaned at once and the whole catalog re-renders on first sight.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Read a stack back from the catalog, or from a hand-written one.
    ///
    /// What comes out has not been through [`EditOp::clamped`] and may hold
    /// values no slider can produce, NaN included, so the renderer defends
    /// itself rather than trusting the range. Geometry may also be in the old
    /// non-canonical shape; the next geometry button normalises it.
    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }
}

/// Address of the developed pixels for this photo.
///
/// Identical to the content hash when nothing has been adjusted, so untouched
/// photos keep every thumbnail that already exists.
pub fn render_key(content_hash: &str, stack: &EditStack) -> String {
    render_key_with_quality(content_hash, stack, DELIVERY_JPEG_QUALITY)
}

/// The key, for a stated encoder quality.
///
/// The cache is addressed by what the pixels *are*, and how they are encoded is
/// part of that: without the quality in the hash, a library that had already
/// rendered a photo would go on serving and publishing bytes from the old
/// encoder for ever, while photos rendered after the change got the new one —
/// two qualities in one delivery, and nothing to show which was which.
fn render_key_with_quality(content_hash: &str, stack: &EditStack, quality: u8) -> String {
    if stack.is_empty() {
        // No edits means the original file itself is the render, so nothing was
        // encoded here and the quality cannot apply.
        return content_hash.to_string();
    }
    let json = stack.to_json().unwrap_or_default();
    blake3::hash(format!("{content_hash}\u{1}{json}\u{1}q{quality}").as_bytes())
        .to_hex()
        .to_string()
}

/// Where a full-size developed render is cached.
///
/// Lives beside the thumbnails, under the same content-addressed sharding, so
/// clearing the derived-data directory clears this too.
pub fn render_cache_path(thumb_root: &Path, render_key: &str) -> PathBuf {
    media::thumb_path(thumb_root, render_key, "full")
}

// ------------------------------------------------------------------ pixels

/// Apply a stack to an image.
///
/// Geometry runs first — cropping before tone means the tone operators see only
/// the pixels that survive, which is what makes a crop-then-adjust workflow
/// behave the way it looks like it should.
pub fn apply(img: &DynamicImage, stack: &EditStack) -> DynamicImage {
    let mut out = img.clone();

    for op in &stack.ops {
        out = match *op {
            EditOp::Rotate { quarter_turns } => match quarter_turns % 4 {
                1 => out.rotate90(),
                2 => out.rotate180(),
                3 => out.rotate270(),
                _ => out,
            },
            EditOp::FlipHorizontal => out.fliph(),
            EditOp::FlipVertical => out.flipv(),
            EditOp::Crop { x, y, w, h } => crop(&out, x, y, w, h),
            _ => out,
        };
    }

    let tone: Vec<&EditOp> = stack
        .ops
        .iter()
        .filter(|op| {
            !matches!(
                op,
                EditOp::Rotate { .. }
                    | EditOp::FlipHorizontal
                    | EditOp::FlipVertical
                    | EditOp::Crop { .. }
            )
        })
        .collect();
    if tone.is_empty() {
        return out;
    }

    let mut rgb = out.to_rgb8();
    for px in rgb.pixels_mut() {
        let mut c = [
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
        ];
        for op in &tone {
            c = apply_tone(c, op);
        }
        px[0] = to_u8(c[0]);
        px[1] = to_u8(c[1]);
        px[2] = to_u8(c[2]);
    }
    DynamicImage::ImageRgb8(rgb)
}

fn crop(img: &DynamicImage, x: f32, y: f32, w: f32, h: f32) -> DynamicImage {
    let (iw, ih) = (img.width(), img.height());
    if iw == 0 || ih == 0 {
        return img.clone();
    }
    // The origin is held one pixel inside the frame. A rectangle dragged flat
    // against the right or bottom edge rounds to an origin *on* that edge,
    // where there is nothing left to take: the width clamp below then asks for
    // one pixel out of zero available and gets zero, and a zero-pixel image is
    // a JPEG the encoder refuses. That refusal came back as the whole album
    // declining to publish, over one frame's crop handle.
    let cx = (clamp_index(x, iw)).min(iw - 1);
    let cy = (clamp_index(y, ih)).min(ih - 1);
    let cw = clamp_index(w, iw).max(1).min(iw - cx);
    let ch = clamp_index(h, ih).max(1).min(ih - cy);
    img.crop_imm(cx, cy, cw, ch)
}

/// A 0..1 fraction of an edge, as a pixel count. Negatives and NaN — which a
/// stack read straight from the catalog has never been clamped against — read
/// as zero rather than wrapping into an enormous `u32`.
fn clamp_index(fraction: f32, edge: u32) -> u32 {
    (fraction * edge as f32).round().max(0.0) as u32
}

/// Rec. 709 luma — the weighting that matches how bright a colour looks.
fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn apply_tone(c: [f32; 3], op: &EditOp) -> [f32; 3] {
    match *op {
        EditOp::Exposure { ev } => {
            let f = 2f32.powf(ev);
            [c[0] * f, c[1] * f, c[2] * f]
        }
        EditOp::Contrast { amount } => {
            // Pivot on mid grey so the image neither darkens nor brightens
            // overall as contrast increases.
            let k = 1.0 + amount / 100.0;
            [
                (c[0] - 0.5) * k + 0.5,
                (c[1] - 0.5) * k + 0.5,
                (c[2] - 0.5) * k + 0.5,
            ]
        }
        EditOp::Saturation { amount } => {
            let k = 1.0 + amount / 100.0;
            let l = luma(c);
            [
                l + (c[0] - l) * k,
                l + (c[1] - l) * k,
                l + (c[2] - l) * k,
            ]
        }
        EditOp::Temperature { amount } => {
            // Warm lifts red and drops blue; a crude but predictable stand-in
            // for a full chromatic-adaptation transform.
            let t = amount / 100.0 * 0.2;
            [c[0] * (1.0 + t), c[1], c[2] * (1.0 - t)]
        }
        EditOp::Tint { amount } => {
            let t = amount / 100.0 * 0.2;
            [c[0] * (1.0 + t * 0.5), c[1] * (1.0 - t), c[2] * (1.0 + t * 0.5)]
        }
        EditOp::Highlights { amount } => {
            // Weight by how bright the pixel already is, so mid-tones and
            // shadows are left where they are.
            let l = luma(c);
            let w = smoothstep(0.5, 1.0, l);
            scale(c, 1.0 + (amount / 100.0) * w * 0.6)
        }
        EditOp::Shadows { amount } => {
            let l = luma(c);
            let w = 1.0 - smoothstep(0.0, 0.5, l);
            scale(c, 1.0 + (amount / 100.0) * w * 0.6)
        }
        EditOp::BlackAndWhite => {
            let l = luma(c);
            [l, l, l]
        }
        _ => c,
    }
}

fn scale(c: [f32; 3], f: f32) -> [f32; 3] {
    [c[0] * f, c[1] * f, c[2] * f]
}

/// Smooth 0→1 ramp between two edges. Avoids the hard seam a linear mask leaves
/// where a highlight or shadow adjustment stops taking effect.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

// ------------------------------------------------------------------ caching

/// Path to the pixels that represent this photo as developed.
///
/// Returns the original file when there are no edits — no copy, no render, no
/// cache entry. Otherwise renders once into the cache and returns that.
pub fn ensure_rendered(
    original: &Path,
    thumb_root: &Path,
    photo: &Photo,
    stack: &EditStack,
) -> Result<PathBuf> {
    if stack.is_empty() {
        return Ok(original.to_path_buf());
    }

    let key = render_key(&photo.content_hash, stack);
    let dest = render_cache_path(thumb_root, &key);
    if dest.exists() {
        return Ok(dest);
    }

    let img = media::load_oriented(original, photo.orientation)?;
    let developed = apply(&img, stack);
    media::write_atomic(&dest, &media::encode_jpeg(&developed, DELIVERY_JPEG_QUALITY)?)?;
    Ok(dest)
}

// ------------------------------------------------------------------ catalog

use crate::catalog::Library;
use rusqlite::{params, OptionalExtension};

impl Library {
    /// The edit stack for one photo. An untouched photo has an empty stack
    /// rather than no stack, so callers never branch on absence.
    pub fn edits(&self, photo_id: i64) -> Result<EditStack> {
        let json: Option<String> = self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT stack_json FROM edits WHERE photo_id = ?1",
                params![photo_id],
                |r| r.get(0),
            )
            .optional()?)
        })?;
        match json {
            Some(text) => EditStack::from_json(&text),
            None => Ok(EditStack::new()),
        }
    }

    /// Store a stack, or delete the row when the stack is empty.
    ///
    /// Deleting matters: an empty row would still make the photo look edited to
    /// anything that checks for the presence of a stack.
    pub fn set_edits(&self, photo_id: i64, stack: &EditStack) -> Result<()> {
        if stack.is_empty() {
            self.with_conn(|c| {
                c.execute("DELETE FROM edits WHERE photo_id = ?1", params![photo_id])?;
                Ok(())
            })?;
            return Ok(());
        }
        let json = stack.to_json()?;
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO edits(photo_id, version, stack_json) VALUES(?1, ?2, ?3) \
                 ON CONFLICT(photo_id) DO UPDATE SET version = ?2, stack_json = ?3",
                params![photo_id, stack.version as i64, json],
            )?;
            Ok(())
        })
    }

    /// Photo ids in this library that carry adjustments.
    pub fn edited_photo_ids(&self) -> Result<Vec<i64>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT photo_id FROM edits ORDER BY photo_id")?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
        })
    }
}

/// Render an adjusted photo and rebuild its thumbnails.
///
/// Called after an edit changes, because the new render key has no derived
/// images yet and the grid would otherwise point at files that do not exist.
/// Cheap when the key has been seen before — reverting an edit usually hits a
/// cache that is still warm.
pub fn render_derived(lib: &Library, photo: &Photo, stack: &EditStack) -> Result<()> {
    let thumb_root = lib.thumb_dir();
    let key = render_key(&photo.content_hash, stack);
    let original = lib.resolve(&photo.rel_path)?;

    // A missing original is not fatal here: the catalog is an index of files
    // that may be on a drive that is not plugged in right now.
    if !original.exists() {
        return Ok(());
    }

    let source = ensure_rendered(&original, &thumb_root, photo, stack)?;
    // Orientation is already baked into a render, and into the original by
    // `load_oriented` — passing it again would rotate twice.
    let orientation = if stack.is_empty() { photo.orientation } else { None };
    media::generate_derived(&source, &thumb_root, &key, orientation)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn flat(r: u8, g: u8, b: u8) -> DynamicImage {
        let mut img = RgbImage::new(4, 4);
        for px in img.pixels_mut() {
            *px = Rgb([r, g, b]);
        }
        DynamicImage::ImageRgb8(img)
    }

    fn px(img: &DynamicImage, x: u32, y: u32) -> [u8; 3] {
        let p = img.to_rgb8();
        let q = p.get_pixel(x, y);
        [q[0], q[1], q[2]]
    }

    fn stack_of(ops: &[EditOp]) -> EditStack {
        let mut s = EditStack::new();
        for op in ops {
            s.set(op.clone());
        }
        s
    }

    #[test]
    fn a_slider_back_at_zero_leaves_no_trace() {
        let mut s = EditStack::new();
        s.set(EditOp::Exposure { ev: 1.0 });
        assert_eq!(s.ops.len(), 1);

        s.set(EditOp::Exposure { ev: 0.0 });
        assert!(s.is_empty(), "an identity op is removed, not stored");

        // …and the photo is back on its original render key, so the thumbnails
        // it already had are still the right ones.
        assert_eq!(render_key("abc123", &s), "abc123");
    }

    #[test]
    fn moving_one_slider_twice_replaces_rather_than_stacks() {
        let mut s = EditStack::new();
        s.set(EditOp::Exposure { ev: 0.5 });
        s.set(EditOp::Exposure { ev: 1.5 });
        s.set(EditOp::Contrast { amount: 20.0 });

        assert_eq!(s.ops.len(), 2);
        assert_eq!(s.get("exposure"), Some(&EditOp::Exposure { ev: 1.5 }));
    }

    #[test]
    fn render_key_changes_with_the_edit_and_comes_back_on_undo() {
        let base = "0123456789abcdef";
        let plain = EditStack::new();
        let bright = stack_of(&[EditOp::Exposure { ev: 1.0 }]);
        let brighter = stack_of(&[EditOp::Exposure { ev: 2.0 }]);

        assert_eq!(render_key(base, &plain), base);
        assert_ne!(render_key(base, &bright), base);
        assert_ne!(render_key(base, &bright), render_key(base, &brighter));

        // Going back to a previous edit returns to a key that is probably still
        // in the cache — this is what makes undo feel instant.
        assert_eq!(render_key(base, &bright), render_key(base, &{
            let mut s = brighter.clone();
            s.set(EditOp::Exposure { ev: 1.0 });
            s
        }));
    }

    #[test]
    fn values_out_of_range_are_clamped_not_rejected() {
        let mut s = EditStack::new();
        s.set(EditOp::Exposure { ev: 99.0 });
        s.set(EditOp::Contrast { amount: -400.0 });
        assert_eq!(s.get("exposure"), Some(&EditOp::Exposure { ev: 5.0 }));
        assert_eq!(s.get("contrast"), Some(&EditOp::Contrast { amount: -100.0 }));
    }

    #[test]
    fn exposure_is_measured_in_stops() {
        let grey = flat(64, 64, 64);
        let up = apply(&grey, &stack_of(&[EditOp::Exposure { ev: 1.0 }]));
        assert_eq!(px(&up, 0, 0), [128, 128, 128], "+1 EV doubles");

        let down = apply(&grey, &stack_of(&[EditOp::Exposure { ev: -1.0 }]));
        assert_eq!(px(&down, 0, 0), [32, 32, 32], "-1 EV halves");
    }

    #[test]
    fn highlights_do_not_disturb_shadows() {
        let dark = flat(20, 20, 20);
        let bright = flat(230, 230, 230);
        let pull = stack_of(&[EditOp::Highlights { amount: -100.0 }]);

        assert_eq!(px(&apply(&dark, &pull), 0, 0), [20, 20, 20], "shadow untouched");
        assert!(
            px(&apply(&bright, &pull), 0, 0)[0] < 200,
            "highlight pulled down"
        );
    }

    #[test]
    fn shadows_do_not_disturb_highlights() {
        let dark = flat(20, 20, 20);
        let bright = flat(230, 230, 230);
        let lift = stack_of(&[EditOp::Shadows { amount: 100.0 }]);

        assert!(px(&apply(&dark, &lift), 0, 0)[0] > 25, "shadow lifted");
        assert_eq!(px(&apply(&bright, &lift), 0, 0), [230, 230, 230], "highlight untouched");
    }

    #[test]
    fn contrast_pivots_on_mid_grey() {
        let mid = flat(128, 128, 128);
        let punchy = apply(&mid, &stack_of(&[EditOp::Contrast { amount: 50.0 }]));
        assert_eq!(px(&punchy, 0, 0), [128, 128, 128], "mid grey is the pivot");

        let dark = apply(&flat(64, 64, 64), &stack_of(&[EditOp::Contrast { amount: 50.0 }]));
        assert!(px(&dark, 0, 0)[0] < 64, "below the pivot goes darker");
    }

    #[test]
    fn black_and_white_keeps_luminance() {
        let red = flat(255, 0, 0);
        let out = apply(&red, &stack_of(&[EditOp::BlackAndWhite]));
        let [r, g, b] = px(&out, 0, 0);
        assert_eq!([r, g, b], [54, 54, 54], "Rec. 709 luma of pure red");
    }

    #[test]
    fn saturation_at_minus_100_is_grey() {
        let red = flat(200, 40, 40);
        let out = apply(&red, &stack_of(&[EditOp::Saturation { amount: -100.0 }]));
        let [r, g, b] = px(&out, 0, 0);
        assert_eq!(r, g);
        assert_eq!(g, b);
    }

    #[test]
    fn geometry_runs_before_tone_so_a_crop_sees_only_what_survives() {
        // Left half black, right half white.
        let mut img = RgbImage::new(4, 2);
        for (x, _y, p) in img.enumerate_pixels_mut() {
            *p = if x < 2 { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) };
        }
        let img = DynamicImage::ImageRgb8(img);

        // Keep the right half, then halve the exposure.
        let s = stack_of(&[
            EditOp::Crop { x: 0.5, y: 0.0, w: 0.5, h: 1.0 },
            EditOp::Exposure { ev: -1.0 },
        ]);
        let out = apply(&img, &s);

        assert_eq!((out.width(), out.height()), (2, 2), "cropped to the right half");
        assert_eq!(px(&out, 0, 0), [128, 128, 128], "white, one stop down");
    }

    /// The button says "rotate right", not "be rotated 90°". Two presses have
    /// to reach 180° — with `set` alone the second replaces the first and the
    /// photo never turns past its first quarter.
    #[test]
    fn rotating_twice_is_a_half_turn() {
        let mut s = EditStack::new();
        s.rotate_by(1);
        s.rotate_by(1);
        assert_eq!(s.get("rotate"), Some(&EditOp::Rotate { quarter_turns: 2 }));
        assert_eq!(s.ops.len(), 1, "one rotation, not a pile of them");
    }

    #[test]
    fn rotating_the_whole_way_round_leaves_no_trace() {
        let mut s = EditStack::new();
        for _ in 0..4 {
            s.rotate_by(1);
        }
        assert!(s.is_empty(), "back where it started, so nothing is stored");
        assert_eq!(render_key("abc123", &s), "abc123", "and the thumbnails still fit");
    }

    #[test]
    fn rotating_left_from_upright_wraps_to_three_quarters() {
        let mut s = EditStack::new();
        s.rotate_by(-1);
        assert_eq!(s.get("rotate"), Some(&EditOp::Rotate { quarter_turns: 3 }));

        // …and one to the right undoes it.
        s.rotate_by(1);
        assert!(s.is_empty());
    }

    /// A crop drawn on a turned frame must move with the photograph when it is
    /// turned again.
    ///
    /// The rectangle is fractions of the frame directly beneath it, so leaving
    /// it where it lay re-reads the same four numbers against swapped axes and
    /// silently reframes the picture — a face framed and then straightened
    /// simply disappears.
    #[test]
    fn a_crop_moves_with_the_frame_it_was_drawn_on() {
        // Top row white, bottom row black, so which half survived is visible.
        let mut img = RgbImage::new(4, 2);
        for (_x, y, p) in img.enumerate_pixels_mut() {
            *p = if y == 0 { Rgb([255, 255, 255]) } else { Rgb([0, 0, 0]) };
        }
        let base = DynamicImage::ImageRgb8(img);

        let mut s = EditStack::new();
        s.rotate_by(1);
        s.set(EditOp::Crop { x: 0.0, y: 0.0, w: 1.0, h: 0.5 });
        let framed = apply(&base, &s);

        s.rotate_by(1);
        let turned = apply(&base, &s);

        let mut quarter = EditStack::new();
        quarter.rotate_by(1);
        let expected = apply(&framed, &quarter);

        assert_eq!(
            (turned.width(), turned.height()),
            (expected.width(), expected.height()),
            "the crop did not move with the turn"
        );
        assert_eq!(
            turned.to_rgb8().as_raw(),
            expected.to_rgb8().as_raw(),
            "the crop landed on a different part of the frame"
        );
    }

    /// A flip has no value to return to zero, so the same button has to take it
    /// off again.
    #[test]
    fn a_flip_toggles_off_with_a_second_press() {
        let mut s = EditStack::new();
        s.toggle(EditOp::FlipHorizontal);
        assert_eq!(s.get("flip-horizontal"), Some(&EditOp::FlipHorizontal));

        s.toggle(EditOp::FlipHorizontal);
        assert!(s.is_empty(), "pressed again, it is gone");

        // The two axes are not independent: mirroring both ways is a half
        // turn, and that is what gets stored, so the photograph can never end
        // up in a state the eight orientations cannot name.
        s.toggle(EditOp::FlipHorizontal);
        s.toggle(EditOp::FlipVertical);

        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(4, 2, |x, y| {
            Rgb([(x * 60) as u8, (y * 120) as u8, 0])
        }));
        assert_eq!(
            apply(&img, &s).to_rgb8().as_raw(),
            apply(&img, &stack_of(&[EditOp::Rotate { quarter_turns: 2 }])).to_rgb8().as_raw(),
            "both mirrors together must be exactly a half turn"
        );
    }

    /// A stack the previous version wrote, with the crop ahead of the turn.
    ///
    /// The photographer framed the shot and then straightened it, so each op
    /// was appended where it fell. Those stacks are in catalogs on disk and
    /// they still render correctly — it is the next press of a geometry button
    /// that has to keep them framing the same part of the picture.
    #[test]
    fn a_crop_recorded_before_the_turn_still_frames_the_same_picture() {
        let base = DynamicImage::ImageRgb8(RgbImage::from_fn(4, 2, |x, y| {
            Rgb([(x * 60) as u8, (y * 120) as u8, 7])
        }));

        for legacy in [
            r#"[{"op":"crop","x":0.0,"y":0.0,"w":1.0,"h":0.5},{"op":"rotate","quarter_turns":1}]"#,
            r#"[{"op":"crop","x":0.25,"y":0.0,"w":0.5,"h":1.0},{"op":"flip-vertical"}]"#,
            r#"[{"op":"crop","x":0.0,"y":0.5,"w":0.5,"h":0.5},{"op":"flip-horizontal"},{"op":"rotate","quarter_turns":3}]"#,
        ] {
            for press in [1i32, -1] {
                let mut s =
                    EditStack::from_json(&format!(r#"{{"version":1,"ops":{legacy}}}"#)).unwrap();
                let before = apply(&base, &s);

                s.rotate_by(press);
                let after = apply(&base, &s);

                let mut quarter = EditStack::new();
                quarter.rotate_by(press);
                let expected = apply(&before, &quarter);

                assert_eq!(
                    (after.width(), after.height()),
                    (expected.width(), expected.height()),
                    "{legacy} turned by {press} reframed the picture"
                );
                assert_eq!(
                    after.to_rgb8().as_raw(),
                    expected.to_rgb8().as_raw(),
                    "{legacy} turned by {press} landed on a different part of the frame"
                );
            }

            // …and the same for a mirror.
            let mut s = EditStack::from_json(&format!(r#"{{"version":1,"ops":{legacy}}}"#)).unwrap();
            let before = apply(&base, &s);
            s.toggle(EditOp::FlipHorizontal);
            let after = apply(&base, &s);
            let expected = apply(&before, &stack_of(&[EditOp::FlipHorizontal]));
            assert_eq!(
                after.to_rgb8().as_raw(),
                expected.to_rgb8().as_raw(),
                "{legacy} mirrored landed on a different part of the frame"
            );
        }
    }

    /// A crop handle dragged flat against an edge.
    ///
    /// There is no such thing as a photograph of no pixels: the JPEG encoder
    /// refuses one, and that refusal reached the photographer as the whole
    /// album declining to publish.
    #[test]
    fn a_crop_flat_against_an_edge_still_leaves_a_picture() {
        let img = flat(90, 90, 90);
        for (x, y, w, h) in [
            (1.0, 0.0, 0.2, 1.0),
            (0.0, 1.0, 1.0, 0.2),
            (1.0, 1.0, 1.0, 1.0),
            (0.99, 0.99, 0.0, 0.0),
        ] {
            let out = apply(&img, &stack_of(&[EditOp::Crop { x, y, w, h }]));
            assert!(
                out.width() >= 1 && out.height() >= 1,
                "crop {x},{y} {w}x{h} left nothing to encode: {:?}",
                (out.width(), out.height())
            );
        }
    }

    #[test]
    fn rotation_swaps_the_axes() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(6, 2));
        let out = apply(&img, &stack_of(&[EditOp::Rotate { quarter_turns: 1 }]));
        assert_eq!((out.width(), out.height()), (2, 6));
    }

    #[test]
    fn an_empty_stack_changes_nothing() {
        let img = flat(37, 99, 211);
        let out = apply(&img, &EditStack::new());
        assert_eq!(px(&out, 0, 0), [37, 99, 211]);
    }

    #[test]
    fn json_round_trips_every_op() {
        let s = stack_of(&[
            EditOp::Exposure { ev: 0.75 },
            EditOp::Contrast { amount: 12.0 },
            EditOp::Saturation { amount: -30.0 },
            EditOp::Temperature { amount: 8.0 },
            EditOp::Tint { amount: -4.0 },
            EditOp::Highlights { amount: -25.0 },
            EditOp::Shadows { amount: 15.0 },
            EditOp::BlackAndWhite,
            EditOp::Rotate { quarter_turns: 3 },
            EditOp::FlipHorizontal,
            EditOp::FlipVertical,
            EditOp::Crop { x: 0.1, y: 0.2, w: 0.5, h: 0.6 },
        ]);
        let back = EditStack::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(s, back);
        assert_eq!(back.version, STACK_VERSION);
    }

    #[test]
    fn a_stack_written_without_a_version_still_loads() {
        // Forward compatibility for anything that hand-writes the field set.
        let s = EditStack::from_json(r#"{"ops":[{"op":"exposure","ev":1.0}]}"#).unwrap();
        assert_eq!(s.version, STACK_VERSION);
        assert_eq!(s.ops.len(), 1);
    }

    /// What a client is sent must not be quietly worse than what was shot.
    ///
    /// An untouched photo is published by copying the camera's own file, so it
    /// keeps whatever quality the body wrote. An adjusted one is re-encoded,
    /// and it went out at the image crate's default of 75 — so moving a single
    /// slider cost the delivered frame real detail, on the one copy the client
    /// actually receives, with nothing anywhere saying so.
    #[test]
    fn a_developed_frame_is_delivered_at_camera_quality() {
        // Fine detail, because that is what a low quality setting destroys and
        // a flat field would hide.
        let mut seed: u64 = 7;
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(320, 240, |x, y| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = ((seed >> 33) & 0x7f) as u32;
            Rgb([((x + n) % 256) as u8, ((y * 2 + n) % 256) as u8, ((x + y + n) % 256) as u8])
        }));

        let encoded = media::encode_jpeg(&img, DELIVERY_JPEG_QUALITY).unwrap();
        let back = image::load_from_memory(&encoded).unwrap();

        let error = mean_abs_error(&img, &back);
        let at_old_default = mean_abs_error(
            &img,
            &image::load_from_memory(&media::encode_jpeg(&img, 75).unwrap()).unwrap(),
        );

        assert!(
            error < at_old_default * 0.75,
            "delivery is no better than the old default: {error:.2} vs {at_old_default:.2}"
        );
    }

    /// The render cache is addressed by the edit stack, not by how the pixels
    /// were encoded — so changing the encoder has to change the key, or a
    /// library that already rendered a photo goes on serving and publishing the
    /// old bytes for ever while new photos get the new ones.
    #[test]
    fn the_render_key_follows_the_delivery_quality() {
        let stack = stack_of(&[EditOp::Exposure { ev: 0.5 }]);
        let key = render_key("abc123", &stack);
        assert!(
            key.contains(char::is_alphanumeric) && key != "abc123",
            "an adjusted photo must not sit on the original's key"
        );
        assert_ne!(
            key,
            render_key_with_quality("abc123", &stack, DELIVERY_JPEG_QUALITY + 1),
            "the key ignored the encoder, so old renders would be served as new"
        );
    }

    fn mean_abs_error(a: &DynamicImage, b: &DynamicImage) -> f64 {
        let (a, b) = (a.to_rgb8(), b.to_rgb8());
        let n = a.as_raw().len() as f64;
        a.as_raw()
            .iter()
            .zip(b.as_raw())
            .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as f64)
            .sum::<f64>()
            / n
    }
}
