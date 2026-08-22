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

/// One adjustment.
///
/// Amounts are the -100..100 scale a slider hands over, except exposure, which
/// is in stops because that is the unit photographers think in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum EditOp {
    /// Stops. +1 doubles the light, -1 halves it.
    Exposure { ev: f32 },
    Contrast { amount: f32 },
    Saturation { amount: f32 },
    /// Positive is warmer.
    Temperature { amount: f32 },
    /// Positive is magenta, negative green.
    Tint { amount: f32 },
    /// Recover blown highlights (negative) or lift them (positive).
    Highlights { amount: f32 },
    Shadows { amount: f32 },
    /// Drop colour, keeping luminance.
    BlackAndWhite,
    /// Quarter turns clockwise, 0..3.
    Rotate { quarter_turns: u8 },
    FlipHorizontal,
    FlipVertical,
    /// Fractions of the frame, each 0..1, after rotation.
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

/// Everything done to one photo, in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EditStack {
    #[serde(default = "default_version")]
    pub version: u32,
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

    /// Turn a further quarter on top of whatever turn is already recorded.
    ///
    /// Rotate buttons are relative — two clicks of "right" mean 180°. Sending
    /// an absolute `Rotate { quarter_turns: 1 }` each time would go nowhere,
    /// because [`set`](Self::set) upserts by kind and the second one would only
    /// replace the first. Wrapping back to zero removes the op, so a photo
    /// turned all the way round is untouched again and keeps its render key.
    pub fn rotate_by(&mut self, quarter_turns: i32) {
        let current = match self.get("rotate") {
            Some(EditOp::Rotate { quarter_turns: turns }) => i32::from(*turns),
            _ => 0,
        };
        self.set(EditOp::Rotate {
            quarter_turns: (current + quarter_turns).rem_euclid(4) as u8,
        });
    }

    /// Switch an op that carries no value on, or off again.
    ///
    /// The flips are the only adjustments with nothing to set to zero, so
    /// `set` alone could never undo one: it drops the existing op and pushes an
    /// identical one straight back.
    pub fn toggle(&mut self, op: EditOp) {
        if self.get(op.kind()).is_some() {
            self.remove(op.kind());
        } else {
            self.set(op);
        }
    }

    pub fn remove(&mut self, kind: &str) {
        self.ops.retain(|op| op.kind() != kind);
    }

    pub fn get(&self, kind: &str) -> Option<&EditOp> {
        self.ops.iter().find(|op| op.kind() == kind)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }
}

/// Address of the developed pixels for this photo.
///
/// Identical to the content hash when nothing has been adjusted, so untouched
/// photos keep every thumbnail that already exists.
pub fn render_key(content_hash: &str, stack: &EditStack) -> String {
    if stack.is_empty() {
        return content_hash.to_string();
    }
    let json = stack.to_json().unwrap_or_default();
    blake3::hash(format!("{content_hash}\u{1}{json}").as_bytes())
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
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let cx = (x * iw).round() as u32;
    let cy = (y * ih).round() as u32;
    let cw = ((w * iw).round() as u32).max(1).min(img.width().saturating_sub(cx).max(1));
    let ch = ((h * ih).round() as u32).max(1).min(img.height().saturating_sub(cy).max(1));
    img.crop_imm(cx, cy, cw, ch)
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
    media::write_atomic(&dest, &media::encode_jpeg(&developed)?)?;
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

    /// The stack is the order the operations run in, so replacing one has to
    /// leave it where it stands. Appending instead slid a second "rotate right"
    /// past an existing crop, and the rectangle the photographer had drawn
    /// landed on a different part of the frame.
    #[test]
    fn adjusting_an_op_again_leaves_it_where_it_stands_in_the_stack() {
        let mut s = EditStack::new();
        s.rotate_by(1);
        s.set(EditOp::Crop { x: 0.0, y: 0.0, w: 1.0, h: 0.5 });
        s.rotate_by(1);

        assert_eq!(
            s.ops,
            vec![
                EditOp::Rotate { quarter_turns: 2 },
                EditOp::Crop { x: 0.0, y: 0.0, w: 1.0, h: 0.5 },
            ],
            "the turn must stay ahead of the crop that was drawn on it"
        );

        // And the pixels follow. Top row white, bottom row black: turned a
        // half, the top of the frame is the old bottom, and keeping the top
        // half of *that* is black.
        let mut img = RgbImage::new(4, 2);
        for (_x, y, p) in img.enumerate_pixels_mut() {
            *p = if y == 0 { Rgb([255, 255, 255]) } else { Rgb([0, 0, 0]) };
        }
        let out = apply(&DynamicImage::ImageRgb8(img), &s);
        assert_eq!((out.width(), out.height()), (4, 1));
        assert_eq!(px(&out, 0, 0), [0, 0, 0], "the crop did not move with the turn");
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

        // The two axes are independent adjustments.
        s.toggle(EditOp::FlipHorizontal);
        s.toggle(EditOp::FlipVertical);
        assert_eq!(s.ops.len(), 2);
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
}
