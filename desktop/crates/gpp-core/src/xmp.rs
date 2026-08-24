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
/// Both are looked for, replaced-extension first, in either case of `xmp`.
pub fn sidecar_for(photo: &Path) -> Option<PathBuf> {
    for ext in ["xmp", "XMP"] {
        let replaced = photo.with_extension(ext);
        if replaced != photo && replaced.is_file() {
            return Some(replaced);
        }
        let mut appended = photo.as_os_str().to_owned();
        appended.push(".");
        appended.push(ext);
        let appended = PathBuf::from(appended);
        if appended.is_file() {
            return Some(appended);
        }
    }
    None
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

        // The appended spelling.
        let appended = dir.path().join("IMG_0001.jpg.xmp");
        std::fs::write(&appended, b"<x/>").unwrap();
        assert_eq!(sidecar_for(&photo), Some(appended.clone()));

        // Lightroom's replaced-extension spelling wins when both exist.
        let replaced = dir.path().join("IMG_0001.xmp");
        std::fs::write(&replaced, b"<x/>").unwrap();
        assert_eq!(sidecar_for(&photo), Some(replaced));
    }
}
