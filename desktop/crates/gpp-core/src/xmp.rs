//! XMP sidecar reading — the few fields this app consumes.
//!
//! Lightroom (and most other editors) can write a `photo.xmp` beside a file
//! instead of touching the file itself. This module reads the handful of
//! fields the catalog cares about: `xmp:Rating`, `xmp:Label`, `dc:subject`
//! (an `rdf:Bag` of keywords) and `tiff:Orientation`. Everything else in the
//! packet — and XMP is deliberately open-ended — is ignored in silence.
//!
//! Both spellings XMP allows are handled: attribute-style, which is what
//! Lightroom emits (`<rdf:Description xmp:Rating="3" …>`), and element-style
//! (`<xmp:Rating>3</xmp:Rating>`). Names are matched by local name, so an
//! unusual namespace prefix does not hide a rating.
//!
//! Nothing here errors on bad input. A sidecar is advisory metadata riding
//! beside the photograph: a truncated or garbled one yields whatever fields
//! were readable before the damage, and an unreadable file yields `None` —
//! the import carries on either way.

use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;

/// The fields read out of one sidecar. Every one is optional, because every
/// one is routinely absent — a sidecar may hold nothing but develop settings
/// this app does not read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XmpSidecar {
    /// `xmp:Rating`, clamped to 0..=5. XMP allows `-1` for "rejected"; that
    /// arrives as rating 0 here — the pick/reject flag is the catalog's own.
    pub rating: Option<u8>,
    /// `xmp:Label`, trimmed; an empty label is no label.
    pub label: Option<String>,
    /// `dc:subject` keywords, in document order, trimmed, blanks dropped.
    /// Case is kept as written; the tag store lowercases on write.
    pub keywords: Vec<String>,
    /// `tiff:Orientation`, kept only when it is a legal EXIF value (1..=8).
    pub orientation: Option<u16>,
}

impl XmpSidecar {
    /// Nothing this app consumes was in the packet.
    pub fn is_empty(&self) -> bool {
        self.rating.is_none()
            && self.label.is_none()
            && self.keywords.is_empty()
            && self.orientation.is_none()
    }
}

/// The sidecar file for a photo, if one exists beside it.
///
/// Lightroom names a sidecar by replacing the extension (`IMG_0001.jpg` →
/// `IMG_0001.xmp`); some other tools append instead (`IMG_0001.jpg.xmp`).
/// Both are looked for, in either case of `xmp`.
///
/// **The appended spelling wins when both exist**, because it is the one that
/// says which file it belongs to. `IMG_0001.xmp` beside a `IMG_0001.NEF` and a
/// `IMG_0001.JPG` cannot describe both — it is exactly the ambiguity
/// [`export_sidecar_for`] refuses to write into — while `IMG_0001.NEF.xmp`
/// names its photograph and nothing else. For the ordinary single-file case
/// nothing changes: no appended sidecar exists, and Lightroom's spelling is
/// found where it always was.
pub fn sidecar_for(photo: &Path) -> Option<PathBuf> {
    for ext in ["xmp", "XMP"] {
        let appended = appended_sidecar(photo, ext);
        if appended.is_file() {
            return Some(appended);
        }
        let replaced = photo.with_extension(ext);
        if replaced != photo && replaced.is_file() {
            return Some(replaced);
        }
    }
    None
}

/// `IMG_0001.NEF` → `IMG_0001.NEF.xmp`: the spelling that names one file.
fn appended_sidecar(photo: &Path, ext: &str) -> PathBuf {
    let mut appended = photo.as_os_str().to_owned();
    appended.push(".");
    appended.push(ext);
    PathBuf::from(appended)
}

/// Read and parse a sidecar. `None` only when the file cannot be read at all;
/// a file that reads but parses badly yields whatever was readable.
pub fn read_sidecar(path: &Path) -> Option<XmpSidecar> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_xmp(&text))
}

/// Which element-style field is currently being read.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Rating,
    Label,
    Orientation,
}

/// Parse an XMP packet. Infallible by design: malformed XML ends the walk and
/// whatever was gathered up to that point is the answer.
pub fn parse_xmp(text: &str) -> XmpSidecar {
    let mut out = XmpSidecar::default();
    let mut reader = Reader::from_str(text);

    // Where in the document we are: inside `dc:subject` (keywords live there,
    // one per `rdf:li`), inside an `rdf:li`, or inside an element-style field.
    let mut subject_depth = 0usize;
    let mut in_li = false;
    let mut capture: Option<Field> = None;

    loop {
        match reader.read_event() {
            // Malformed input: keep what was readable, stop looking.
            Err(_) => break,
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                read_attributes(&mut out, &e);
                let name = e.name();
                match local_name(name.as_ref()) {
                    "subject" => subject_depth += 1,
                    "li" if subject_depth > 0 => in_li = true,
                    "Rating" => capture = Some(Field::Rating),
                    "Label" => capture = Some(Field::Label),
                    "Orientation" => capture = Some(Field::Orientation),
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => read_attributes(&mut out, &e),

            Ok(Event::Text(t)) => {
                let Ok(text) = t.unescape() else { continue };
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                if in_li && subject_depth > 0 {
                    out.keywords.push(text.to_string());
                } else if let Some(field) = capture {
                    apply(&mut out, field, text);
                }
            }

            Ok(Event::End(e)) => {
                let name = e.name();
                match local_name(name.as_ref()) {
                    "subject" => subject_depth = subject_depth.saturating_sub(1),
                    "li" => in_li = false,
                    _ => {}
                }
                capture = None;
            }

            // Comments, CDATA, processing instructions, doctype: not ours.
            Ok(_) => {}
        }
    }
    out
}

/// Attribute-style fields, which is how Lightroom writes them: everything sits
/// as attributes on one `rdf:Description`.
fn read_attributes(out: &mut XmpSidecar, e: &quick_xml::events::BytesStart<'_>) {
    for attr in e.attributes().flatten() {
        let field = match local_name(attr.key.as_ref()) {
            "Rating" => Field::Rating,
            "Label" => Field::Label,
            "Orientation" => Field::Orientation,
            _ => continue,
        };
        if let Ok(value) = attr.unescape_value() {
            apply(out, field, value.trim());
        }
    }
}

/// Store one parsed value. First one wins — a packet with two `rdf:Description`
/// blocks keeps the earlier rating rather than whichever came last.
fn apply(out: &mut XmpSidecar, field: Field, value: &str) {
    match field {
        Field::Rating => {
            if out.rating.is_none() {
                // XMP ratings are nominally integers but arrive as "3.0" from
                // some writers, and as -1 for "rejected".
                if let Ok(v) = value.parse::<f32>() {
                    if v.is_finite() {
                        out.rating = Some(v.clamp(0.0, 5.0).round() as u8);
                    }
                }
            }
        }
        Field::Label => {
            if out.label.is_none() && !value.is_empty() {
                out.label = Some(value.to_string());
            }
        }
        Field::Orientation => {
            if out.orientation.is_none() {
                if let Ok(v) = value.parse::<u16>() {
                    if (1..=8).contains(&v) {
                        out.orientation = Some(v);
                    }
                }
            }
        }
    }
}

/// The part of a qualified name after the prefix: `xmp:Rating` → `Rating`.
fn local_name(qname: &[u8]) -> &str {
    let local = match qname.iter().rposition(|b| *b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    };
    std::str::from_utf8(local).unwrap_or("")
}

// ---------------------------------------------------------------- exporting

/// The `gpp:` namespace URI. Its presence in a sidecar is the marker that the
/// file is ours to overwrite; a sidecar without it belongs to another tool
/// and is left strictly alone.
pub const GPP_NS: &str = "https://goldplated.photos/ns/gpp/1.0/";

/// Version of the exported develop-stack payload — the stack's own
/// `version` field travels inside the JSON; this one versions the envelope.
pub const GPP_XMP_VERSION: u32 = 1;

/// What one photo's export did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportedSidecar {
    /// A sidecar was written (created, or an earlier gpp one replaced).
    Written(PathBuf),
    /// A sidecar from another tool sits where ours would go. Untouched.
    ForeignKept(PathBuf),
}

/// Outcome of exporting sidecars for a set of photos.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct XmpExportOutcome {
    /// Sidecars written.
    pub written: usize,
    /// Sidecars another tool wrote, left exactly as they were — the
    /// photographer decides what happens to those, not an export.
    pub skipped_foreign: Vec<String>,
    /// Photos whose original is gone from disk, so there is nowhere sensible
    /// to put a sidecar.
    pub missing: Vec<String>,
    /// Photos on a source that is not reachable right now. Nothing is wrong
    /// with them; the drive is elsewhere, and the next export writes their
    /// sidecars beside them. Each entry names the source.
    pub offline: Vec<String>,
}

/// Render one sidecar packet.
///
/// Standard fields ride in the vocabularies every serious tool reads —
/// `xmp:Rating`, `xmp:Label`, `dc:subject`, `tiff:Orientation`, written the
/// attribute-plus-bag way Lightroom writes them (and [`parse_xmp`] reads
/// back). The develop stack has no public vocabulary, so it travels under the
/// `gpp:` namespace as the stack JSON verbatim: readable by us on any
/// machine, harmlessly opaque to everyone else.
pub fn render_sidecar(
    rating: u8,
    label: Option<&str>,
    keywords: &[String],
    orientation: Option<u16>,
    stack_json: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n");
    out.push_str("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n");
    out.push_str(" <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    out.push_str("  <rdf:Description rdf:about=\"\"\n");
    out.push_str("    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n");
    out.push_str("    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n");
    out.push_str("    xmlns:tiff=\"http://ns.adobe.com/tiff/1.0/\"\n");
    out.push_str(&format!("    xmlns:gpp=\"{GPP_NS}\"\n"));
    out.push_str(&format!("    gpp:Version=\"{GPP_XMP_VERSION}\"\n"));
    if rating > 0 {
        out.push_str(&format!("    xmp:Rating=\"{rating}\"\n"));
    }
    if let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) {
        out.push_str(&format!("    xmp:Label=\"{}\"\n", xml_escape(label)));
    }
    if let Some(o) = orientation.filter(|o| (1..=8).contains(o)) {
        out.push_str(&format!("    tiff:Orientation=\"{o}\"\n"));
    }
    out.push_str("    >\n");
    if !keywords.is_empty() {
        out.push_str("   <dc:subject>\n    <rdf:Bag>\n");
        for k in keywords {
            out.push_str(&format!("     <rdf:li>{}</rdf:li>\n", xml_escape(k)));
        }
        out.push_str("    </rdf:Bag>\n   </dc:subject>\n");
    }
    if let Some(stack) = stack_json {
        out.push_str(&format!(
            "   <gpp:DevelopStack>{}</gpp:DevelopStack>\n",
            xml_escape(stack)
        ));
    }
    out.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n");
    out.push_str("<?xpacket end=\"w\"?>");
    out
}

/// Whether a sidecar's text carries the gpp marker — i.e. whether we wrote it.
pub fn is_gpp_sidecar(text: &str) -> bool {
    text.contains(GPP_NS)
}

/// Read the develop stack JSON back out of a sidecar's `gpp:DevelopStack`
/// element. `None` when the packet has none — a foreign sidecar, or an
/// untouched frame's.
pub fn read_gpp_stack(text: &str) -> Option<String> {
    let mut reader = Reader::from_str(text);
    let mut capturing = false;
    let mut captured = String::new();
    loop {
        match reader.read_event() {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == "DevelopStack" {
                    capturing = true;
                }
            }
            Ok(Event::Text(t)) if capturing => {
                if let Ok(text) = t.unescape() {
                    captured.push_str(&text);
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == "DevelopStack" && capturing {
                    let trimmed = captured.trim().to_string();
                    return (!trimmed.is_empty()).then_some(trimmed);
                }
            }
            Ok(_) => {}
        }
    }
    None
}

/// Where an export writes a photo's sidecar, honouring what already exists.
///
/// **The rule.** A photograph gets the bare-stem sidecar Lightroom writes and
/// looks for — `IMG_0001.NEF` → `IMG_0001.xmp` — *unless* another photograph
/// beside it shares that stem, in which case every one of them gets the
/// full-filename form instead: `IMG_0001.NEF.xmp`, `IMG_0001.JPG.xmp`. That is
/// `shares_stem`, and the caller works it out from the set it is exporting.
///
/// A RAW+JPEG pair is two rows in this catalog, with their own ratings, labels
/// and develop stacks, and one bare-stem sidecar cannot hold two of those: it
/// held whichever was written second, silently, and the export counted both as
/// written. Neither photo gets the ambiguous name, rather than one of them
/// winning it by import order — a rule that would move the RAW's sidecar the
/// day a JPEG landed next to it. Interop for the common single-file case is
/// untouched, which is the case Lightroom's convention exists for, and
/// [`sidecar_for`] reads both spellings anyway.
///
/// Whatever the name works out to, an existing packet there decides what
/// happens to it: ours is overwritten in place, one from another tool is kept
/// and returned as `Err` for the caller to report.
pub fn export_sidecar_for(
    photo_path: &Path,
    shares_stem: bool,
) -> std::result::Result<PathBuf, PathBuf> {
    let dest = if shares_stem {
        appended_sidecar(photo_path, "xmp")
    } else if let Some(existing) = sidecar_for(photo_path) {
        existing
    } else {
        let replaced = photo_path.with_extension("xmp");
        // An extensionless photo: append instead of replacing nothing.
        if replaced == photo_path {
            appended_sidecar(photo_path, "xmp")
        } else {
            replaced
        }
    };

    // A file that cannot be read is not evidence of another tool's work — an
    // absent one above all, which is the ordinary case.
    match std::fs::read_to_string(&dest) {
        Ok(text) if !is_gpp_sidecar(&text) => Err(dest),
        _ => Ok(dest),
    }
}

/// Write a sidecar so that an interrupted write cannot destroy the previous one.
///
/// [`std::fs::write`] truncates before it writes, so a crash, a full disk or a
/// pulled drive between the truncate and the last byte leaves a fragment — and
/// a fragment carries no `gpp:` namespace, so [`is_gpp_sidecar`] then reads it
/// as another tool's file and every later export refuses to repair it, for
/// good. The packet goes to a temp name beside the destination and is renamed
/// into place, so the sidecar is only ever the old packet or the new one.
///
/// Not [`crate::media::write_atomic`]: that one creates the destination's
/// parent (a sidecar's parent is the photograph's own folder, and conjuring one
/// would mean the original is not where the catalog says) and randomises the
/// temp name for the many threads that write the thumbnail cache. Sidecar
/// export is one photo at a time, and a predictable leftover is one a
/// photographer can recognise.
fn write_sidecar(dest: &Path, packet: &str) -> crate::error::Result<()> {
    let mut temp = dest.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, packet).map_err(|e| crate::error::Error::io(&temp, e))?;
    match std::fs::rename(&temp, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(crate::error::Error::io(dest, e))
        }
    }
}

/// Photos whose bare-stem sidecar would name more than one of them.
///
/// Keyed by folder and stem, both taken from the path the sidecar would sit
/// beside — a referenced photo's stem is shared with what is on *its* drive,
/// not with a same-named frame gathered into the library.
fn shared_stems(paths: &[PathBuf]) -> std::collections::BTreeSet<PathBuf> {
    let mut seen: std::collections::BTreeMap<PathBuf, usize> = std::collections::BTreeMap::new();
    for path in paths {
        *seen.entry(stem_key(path)).or_default() += 1;
    }
    seen.into_iter().filter(|(_, n)| *n > 1).map(|(k, _)| k).collect()
}

/// The folder-plus-stem a bare-stem sidecar is named after.
fn stem_key(photo: &Path) -> PathBuf {
    let mut key = photo.to_path_buf();
    key.set_extension("");
    key
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

impl crate::catalog::Library {
    /// Write (or refresh) XMP sidecars for one album's photos, or for the
    /// whole catalog when `album_path` is `None`.
    ///
    /// The catalog stays the source of truth; sidecars are regenerated from it
    /// so triage work survives a move to any other tool, and so a library is
    /// reconstructible from its files alone. They do **not** ride along in a
    /// `full`-scope sync: that namespace is built from catalogued photos
    /// (`full::local_full_side`), and a `.xmp` is not one — it is neither
    /// catalogued nor a member of any album, so no manifest on either side ever
    /// mentions it. Carrying them would be a change to what the namespace
    /// holds, not a comment.
    ///
    /// The image file itself is never touched, and a sidecar another tool wrote
    /// — anything without the `gpp:` namespace marker — is left alone and named
    /// in the outcome.
    pub fn export_xmp(&self, album_path: Option<&str>) -> crate::error::Result<XmpExportOutcome> {
        let photos = match album_path {
            Some(path) => {
                // Refuse a path that names nothing, rather than exporting an
                // empty success.
                if self.album_by_path(path)?.is_none() {
                    return Err(crate::error::Error::AlbumNotFound(path.to_string()));
                }
                self.album_photos(path)?
            }
            None => self.photos(&crate::model::PhotoFilter::default())?,
        };

        let mut out = XmpExportOutcome::default();

        // Resolve every original first: which sidecar name a photo gets depends
        // on the others beside it, so the set has to be known before the first
        // file is written. "Beside it" means the photographs of this export —
        // the ones this catalog actually has metadata for, and so the only ones
        // that can collide. A sidecar goes beside the original, wherever the
        // original is — a referenced photo's sidecar is written on its own
        // drive, not gathered into the library. A drive that is not attached is
        // named and skipped: one unplugged source must not abandon the export
        // of everything else.
        let mut targets = Vec::with_capacity(photos.len());
        for photo in &photos {
            let original = match self.photo_path(photo) {
                Ok(p) => p,
                Err(crate::error::Error::SourceOffline { name, .. }) => {
                    out.offline.push(format!(
                        "{} (source '{name}' is not available)",
                        photo.rel_path
                    ));
                    continue;
                }
                Err(e) => return Err(e),
            };
            if !original.exists() {
                out.missing.push(photo.rel_path.clone());
                continue;
            }
            targets.push((photo, original));
        }

        let shared = shared_stems(&targets.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>());
        for (photo, original) in &targets {
            let dest = match export_sidecar_for(original, shared.contains(&stem_key(original))) {
                Ok(dest) => dest,
                Err(foreign) => {
                    out.skipped_foreign.push(foreign.display().to_string());
                    continue;
                }
            };
            let stack = self.edits(photo.id)?;
            let stack_json = if stack.is_empty() {
                None
            } else {
                Some(stack.to_json()?)
            };
            let packet = render_sidecar(
                photo.rating,
                photo.color_label.as_deref(),
                &self.photo_tags(photo.id)?,
                photo.orientation,
                stack_json.as_deref(),
            );
            write_sidecar(&dest, &packet)?;
            out.written += 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Lightroom actually writes: one rdf:Description carrying the scalar
    /// fields as attributes, with the keyword bag as a child element.
    #[test]
    fn attribute_style_as_lightroom_writes_it() {
        let xmp = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:tiff="http://ns.adobe.com/tiff/1.0/"
    xmp:Rating="4"
    xmp:Label="Red"
    tiff:Orientation="6"
    xmp:CreatorTool="Adobe Photoshop Lightroom Classic">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>Wedding</rdf:li>
     <rdf:li> Bride </rdf:li>
     <rdf:li></rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;
        let parsed = parse_xmp(xmp);
        assert_eq!(parsed.rating, Some(4));
        assert_eq!(parsed.label.as_deref(), Some("Red"));
        assert_eq!(parsed.orientation, Some(6));
        assert_eq!(parsed.keywords, vec!["Wedding", "Bride"], "trimmed, blanks dropped");
    }

    #[test]
    fn element_style_is_read_too() {
        let xmp = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/"
                   xmlns:tiff="http://ns.adobe.com/tiff/1.0/">
    <xmp:Rating>3</xmp:Rating>
    <xmp:Label>Blue</xmp:Label>
    <tiff:Orientation>8</tiff:Orientation>
  </rdf:Description>
</rdf:RDF>"#;
        let parsed = parse_xmp(xmp);
        assert_eq!(parsed.rating, Some(3));
        assert_eq!(parsed.label.as_deref(), Some("Blue"));
        assert_eq!(parsed.orientation, Some(8));
        assert!(parsed.keywords.is_empty());
    }

    /// XMP is open-ended, so a packet full of things this app has never heard
    /// of has to parse down to "nothing for us" rather than fail — and values
    /// that are out of range are dropped, not clamped into meaning.
    #[test]
    fn unknown_content_and_bad_values_are_tolerated_in_silence() {
        let parsed = parse_xmp(
            r#"<rdf:RDF xmlns:rdf="r"><rdf:Description
                 xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
                 crs:Exposure2012="+0.5" crs:ToneCurveName2012="Linear">
                 <crs:GradientBasedCorrections><rdf:Seq><rdf:li/></rdf:Seq>
                 </crs:GradientBasedCorrections>
               </rdf:Description></rdf:RDF>"#,
        );
        assert!(parsed.is_empty());

        // A rejected flag (-1) is not a star; a 9 is not an orientation.
        let odd = parse_xmp(
            r#"<r xmlns:xmp="x" xmlns:tiff="t"><d xmp:Rating="-1" tiff:Orientation="9"
                 xmp:Label="  "/></r>"#,
        );
        assert_eq!(odd.rating, Some(0), "XMP's -1 'rejected' clamps to unrated");
        assert_eq!(odd.orientation, None);
        assert_eq!(odd.label, None);

        // Ratings written as decimals still land.
        assert_eq!(parse_xmp(r#"<r xmlns:xmp="x"><d xmp:Rating="3.0"/></r>"#).rating, Some(3));
    }

    /// Truncated or plainly-not-XML input must never panic or error — the
    /// answer is whatever was readable before the damage.
    #[test]
    fn garbage_yields_what_was_readable_and_never_panics() {
        assert!(parse_xmp("").is_empty());
        assert!(parse_xmp("this is not xml at all").is_empty());
        assert!(parse_xmp("<unclosed <<>> &&& garbage").is_empty());

        // Damage after the fields: the fields survive.
        let truncated = parse_xmp(
            r#"<rdf:RDF xmlns:rdf="r"><rdf:Description xmlns:xmp="x" xmp:Rating="5">
               <dc:subject xmlns:dc="d"><rdf:Bag><rdf:li>Keeper</rdf:li"#,
        );
        assert_eq!(truncated.rating, Some(5));
    }

    #[test]
    fn sidecars_are_found_by_either_naming_convention() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("IMG_0001.jpg");
        std::fs::write(&photo, b"jpeg").unwrap();
        assert_eq!(sidecar_for(&photo), None);

        // Lightroom's replaced-extension spelling, on its own.
        let replaced = dir.path().join("IMG_0001.xmp");
        std::fs::write(&replaced, b"<x/>").unwrap();
        assert_eq!(sidecar_for(&photo), Some(replaced));

        // The appended spelling wins when both exist: it names one file, while
        // the bare stem could belong to any frame sharing it — which is exactly
        // the case an export writes the appended form for.
        let appended = dir.path().join("IMG_0001.jpg.xmp");
        std::fs::write(&appended, b"<x/>").unwrap();
        assert_eq!(sidecar_for(&photo), Some(appended));
    }

    /// The written packet has to be one [`parse_xmp`] — this module's own
    /// reader, and the shape Lightroom reads — gets everything back out of.
    #[test]
    fn an_exported_sidecar_round_trips_through_the_reader() {
        let stack = r#"{"version":1,"ops":[{"kind":"exposure","ev":0.5}]}"#;
        let packet = render_sidecar(
            4,
            Some("Red & \"loud\""),
            &["Wedding".to_string(), "Bride <3".to_string()],
            Some(6),
            Some(stack),
        );

        let parsed = parse_xmp(&packet);
        assert_eq!(parsed.rating, Some(4));
        assert_eq!(parsed.label.as_deref(), Some("Red & \"loud\""));
        assert_eq!(parsed.keywords, vec!["Wedding", "Bride <3"]);
        assert_eq!(parsed.orientation, Some(6));

        assert!(is_gpp_sidecar(&packet));
        assert_eq!(read_gpp_stack(&packet).as_deref(), Some(stack));

        // Untouched frame: no rating, no stack — still a valid marked packet.
        let bare = render_sidecar(0, None, &[], None, None);
        assert!(parse_xmp(&bare).is_empty());
        assert!(is_gpp_sidecar(&bare));
        assert_eq!(read_gpp_stack(&bare), None);
    }

    /// A sidecar without our namespace belongs to another tool. Ours may
    /// never overwrite it — that file can hold develop work this app cannot
    /// even represent.
    #[test]
    fn a_foreign_sidecar_is_kept_and_a_gpp_one_is_replaced_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("IMG_0001.jpg");
        std::fs::write(&photo, b"jpeg").unwrap();

        // Nothing there yet: Lightroom's replaced-extension convention.
        assert_eq!(
            export_sidecar_for(&photo, false),
            Ok(dir.path().join("IMG_0001.xmp"))
        );

        // A foreign sidecar sits there: refused, named.
        let foreign = dir.path().join("IMG_0001.xmp");
        std::fs::write(&foreign, "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>").unwrap();
        assert_eq!(export_sidecar_for(&photo, false), Err(foreign.clone()));

        // One of ours: overwritten in place.
        std::fs::write(&foreign, render_sidecar(3, None, &[], None, None)).unwrap();
        assert_eq!(export_sidecar_for(&photo, false), Ok(foreign.clone()));

        // With the stem shared, the bare name is not a candidate at all — not
        // even the foreign packet sitting on it, which stays exactly where it
        // is while this photograph gets a name of its own.
        assert_eq!(
            export_sidecar_for(&photo, true),
            Ok(dir.path().join("IMG_0001.jpg.xmp"))
        );
    }

    /// A RAW and a JPEG of the same frame are two catalog rows with two sets of
    /// metadata, and `IMG_0001.xmp` can only hold one of them: the second
    /// export overwrote the first's packet and both were counted as written, so
    /// a photographer moving to another tool silently lost half their triage.
    #[test]
    fn a_raw_and_jpeg_pair_keep_separate_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        // The JPEG is a real one so the import catalogues it; the NEF is bytes
        // — never decoded, catalogued by extension, which is the point.
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(24, 16)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        std::fs::write(dir.path().join("IMG_0001.jpg"), buf.into_inner()).unwrap();
        std::fs::write(dir.path().join("IMG_0001.nef"), b"raw sensor bytes").unwrap();
        // A third frame, alone on its stem: it must keep Lightroom's spelling.
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(20, 20)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        std::fs::write(dir.path().join("IMG_0002.jpg"), buf.into_inner()).unwrap();

        let lib = crate::catalog::Library::open(dir.path()).unwrap();
        crate::import::import_dir(&lib, dir.path(), &Default::default(), None, None).unwrap();

        let jpg = lib.photo_by_rel_path("IMG_0001.jpg").unwrap().unwrap();
        let nef = lib.photo_by_rel_path("IMG_0001.nef").unwrap().unwrap();
        lib.set_rating(jpg.id, 2).unwrap();
        lib.set_color_label(jpg.id, Some("Blue")).unwrap();
        lib.set_rating(nef.id, 5).unwrap();
        lib.set_color_label(nef.id, Some("Red")).unwrap();

        let out = lib.export_xmp(None).unwrap();
        assert_eq!(out.written, 3, "one sidecar per photograph");
        assert!(out.skipped_foreign.is_empty(), "{:?}", out.skipped_foreign);

        // The pair share a stem, so neither takes the ambiguous name…
        assert!(!dir.path().join("IMG_0001.xmp").exists());
        let from_jpg = read_sidecar(&dir.path().join("IMG_0001.jpg.xmp")).unwrap();
        let from_nef = read_sidecar(&dir.path().join("IMG_0001.nef.xmp")).unwrap();
        assert_eq!((from_jpg.rating, from_jpg.label.as_deref()), (Some(2), Some("Blue")));
        assert_eq!((from_nef.rating, from_nef.label.as_deref()), (Some(5), Some("Red")));

        // …and each reads back to its own photograph, not to the other's.
        assert_eq!(
            sidecar_for(&dir.path().join("IMG_0001.nef")),
            Some(dir.path().join("IMG_0001.nef.xmp"))
        );

        // The lone frame keeps the bare stem every other tool looks for.
        assert_eq!(
            sidecar_for(&dir.path().join("IMG_0002.jpg")),
            Some(dir.path().join("IMG_0002.xmp"))
        );

        // Re-running is idempotent: same three names, nothing multiplied.
        let again = lib.export_xmp(None).unwrap();
        assert_eq!(again.written, 3);
        let xmps: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".xmp"))
            .collect();
        assert_eq!(
            xmps,
            ["IMG_0001.jpg.xmp", "IMG_0001.nef.xmp", "IMG_0002.xmp"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
    }

    /// A write that cannot finish must leave the previous sidecar whole.
    ///
    /// `std::fs::write` truncates first, so a crash or a full disk part-way
    /// leaves a fragment — and a fragment carries no `gpp:` namespace, so
    /// `is_gpp_sidecar` reads it as another tool's file and every later export
    /// refuses to repair it, permanently. The write goes through a temp file
    /// and a rename, so the destination only ever holds the old packet or the
    /// new one.
    #[test]
    fn an_interrupted_sidecar_write_leaves_the_previous_one_intact() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("IMG_0001.xmp");
        let before = render_sidecar(3, Some("Red"), &["keeper".to_string()], Some(6), None);
        write_sidecar(&dest, &before).unwrap();

        // The replacement cannot be staged: the temp name is taken by a
        // directory, which no write and no rename can replace. Same shape as
        // the disk filling up or the process being killed mid-write.
        std::fs::create_dir(dir.path().join("IMG_0001.xmp.tmp")).unwrap();
        let interrupted = write_sidecar(&dest, &render_sidecar(5, None, &[], None, None));
        assert!(
            interrupted.is_err(),
            "the replacement went straight onto the destination — a truncating write \
             has nothing to interrupt, and so nothing to keep whole"
        );

        let after = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(after, before, "the previous packet was truncated");
        assert!(
            is_gpp_sidecar(&after),
            "a half-written packet reads as another tool's file, and is then never repaired"
        );
        assert_eq!(read_sidecar(&dest).unwrap().rating, Some(3));
    }

    /// End to end through the catalog: fields land in the sidecar, a foreign
    /// sidecar survives untouched and is reported, and the image file itself
    /// is byte-identical afterwards.
    #[test]
    fn export_xmp_writes_fields_and_never_touches_foreign_files_or_originals() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.jpg", "b.jpg"] {
            let mut buf = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(24, 16)
                .write_to(&mut buf, image::ImageFormat::Jpeg)
                .unwrap();
            std::fs::write(dir.path().join(name), buf.into_inner()).unwrap();
        }
        let lib = crate::catalog::Library::open(dir.path()).unwrap();
        crate::import::import_dir(&lib, dir.path(), &Default::default(), None, None).unwrap();

        let a = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();
        let b = lib.photo_by_rel_path("b.jpg").unwrap().unwrap();
        lib.set_rating(a.id, 5).unwrap();
        lib.set_color_label(a.id, Some("Red")).unwrap();
        lib.set_photo_tags(a.id, &["wedding".to_string(), "bride".to_string()]).unwrap();
        let mut stack = lib.edits(a.id).unwrap();
        stack.set(crate::develop::EditOp::Exposure { ev: 0.3 });
        lib.set_edits(a.id, &stack).unwrap();

        // b already has a sidecar from another tool.
        std::fs::write(dir.path().join("b.xmp"), "<foreign/>").unwrap();
        let original_bytes = std::fs::read(dir.path().join("a.jpg")).unwrap();

        let out = lib.export_xmp(None).unwrap();
        assert_eq!(out.written, 1);
        assert_eq!(out.skipped_foreign.len(), 1);
        assert!(out.skipped_foreign[0].ends_with("b.xmp"));

        assert_eq!(
            std::fs::read(dir.path().join("a.jpg")).unwrap(),
            original_bytes,
            "the image file itself was touched"
        );
        assert_eq!(std::fs::read_to_string(dir.path().join("b.xmp")).unwrap(), "<foreign/>");

        let sidecar = read_sidecar(&dir.path().join("a.xmp")).unwrap();
        assert_eq!(sidecar.rating, Some(5));
        assert_eq!(sidecar.label.as_deref(), Some("Red"));
        assert_eq!(sidecar.keywords, vec!["bride", "wedding"]);
        let text = std::fs::read_to_string(dir.path().join("a.xmp")).unwrap();
        assert_eq!(
            read_gpp_stack(&text).as_deref(),
            Some(lib.edits(a.id).unwrap().to_json().unwrap().as_str()),
            "the develop stack rides verbatim under the gpp namespace"
        );

        // An album path that names nothing is refused, not an empty success.
        assert!(lib.export_xmp(Some("no/such/album")).is_err());
        let _ = b;
    }
}
