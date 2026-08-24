//! Domain types shared across the core and exposed to the shell.

use serde::{Deserialize, Serialize};

/// What kind of media a catalog entry is.
///
/// `Raw` is catalogued in phase A (metadata + embedded preview) but not
/// developed; see `media::RawDecoder`.
///
/// Decided from the file extension alone by `media::classify`, at import time —
/// nothing sniffs the bytes. A file misnamed by whatever wrote it is classified
/// by the lie and only found out when something tries to decode it, at which
/// point it is catalogued anyway and listed in `ImportSummary::undecodable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhotoKind {
    /// A still the built-in decoder can open end to end: JPEG, PNG, WebP, TIFF.
    /// The only kind develop and publish render pixels for.
    Photo,
    /// A camera negative. The catalog holds its metadata and, where the shell
    /// supplied a decoder, an embedded preview — but with the default
    /// `media::NullRawDecoder` there are no pixels, and the frame appears in the
    /// grid without a thumbnail rather than not at all.
    Raw,
    /// A clip. Carried through import, albums and publish as a file, never
    /// decoded, never developed: the gallery plays it in the browser.
    Video,
}

impl PhotoKind {
    /// The lowercase string stored in the `photos.kind` column and sent over
    /// the IPC/FFI boundary. Changing one of these renames a value in every
    /// existing catalog, so they are written out rather than derived.
    pub fn as_str(self) -> &'static str {
        match self {
            PhotoKind::Photo => "photo",
            PhotoKind::Raw => "raw",
            PhotoKind::Video => "video",
        }
    }

    /// Parse from the string form stored in the database (infallible; unknown
    /// values fall back to the default variant).
    pub fn parse(s: &str) -> Self {
        match s {
            "raw" => PhotoKind::Raw,
            "video" => PhotoKind::Video,
            _ => PhotoKind::Photo,
        }
    }
}

/// Pick / reject flag, the Lightroom-style triage primitive.
///
/// A flag never destroys anything: no file is deleted, no catalog row removed,
/// no photo dropped from an album. A reject is undone with one keystroke and a
/// deleted negative is not, so the two acts are kept apart.
///
/// It is not inert either, and this is the part to know before culling in bulk:
/// **publishing skips rejected photos by default**
/// (`publish::PublishOptions::exclude_rejected`, which is `true` unless a caller
/// says otherwise), regardless of rating. So a `Reject` does decide what the
/// client sees in their gallery — it just cannot cost the photographer the
/// frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    /// Not yet triaged. The state every imported frame starts in, and what
    /// clearing a flag returns it to — indistinguishable from "looked at and
    /// had no opinion".
    None,
    Pick,
    Reject,
}

impl Flag {
    /// The lowercase string stored in the `photos.flag` column, and the one
    /// [`PhotoFilter::flag`] is compared against in SQL. Same contract as
    /// [`PhotoKind::as_str`]: these strings live in catalogs on disk.
    pub fn as_str(self) -> &'static str {
        match self {
            Flag::None => "none",
            Flag::Pick => "pick",
            Flag::Reject => "reject",
        }
    }

    /// Parse from the string form stored in the database (infallible; unknown
    /// values fall back to the default variant).
    pub fn parse(s: &str) -> Self {
        match s {
            "pick" => Flag::Pick,
            "reject" => Flag::Reject,
            _ => Flag::None,
        }
    }
}

/// A catalogued file — one row of `photos`, as the rest of the app sees it.
///
/// Everything here is *derived*: the file on disk is the truth, and this is what
/// the import pipeline read out of it. Delete the catalog and re-importing
/// rebuilds every field below except the three a person put there by hand —
/// [`rating`](Self::rating), [`flag`](Self::flag), [`color_label`](Self::color_label).
/// Those are the ones worth backing up.
///
/// The camera fields are all `Option` because EXIF is optional and frequently
/// wrong: a scan has none, a phone omits the lens, a second shooter's body
/// writes a make and no model. `None` means "the file did not say", never "zero"
/// — a filter that treats a missing ISO as 0 hides the frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    /// SQLite rowid. Stable while the row lives, and nothing more: a photo
    /// pruned by [`Library::prune_missing`](crate::Library::prune_missing) and
    /// re-imported gets whatever rowid SQLite hands out next, which may be one
    /// another photograph used to hold. Nothing outside the catalog should
    /// persist an id — across runs, address a photo by `rel_path`.
    pub id: i64,
    /// Path relative to the library root, always '/'-separated.
    ///
    /// Unique in the catalog, and the reason importing a folder from outside the
    /// library copies it in first: there is no way to express a photo that lives
    /// somewhere else.
    pub rel_path: String,
    /// Last segment of `rel_path`. Duplicated out of it because it is what the
    /// gallery publishes as the file's name and what free-text search matches.
    pub filename: String,
    /// BLAKE3 of the file's bytes, hex. Identifies the *negative*: two paths
    /// with the same hash are the same photograph imported twice, and the render
    /// key that names a developed image is built from this plus the edit stack,
    /// so an untouched photo's cached render is found by this value alone.
    pub content_hash: String,
    /// Size in bytes, as the filesystem reported it at import.
    pub file_size: i64,
    /// Modification time in milliseconds since the Unix epoch. Paired with
    /// `file_size` this is the cheap "has it changed?" test the import uses to
    /// skip a file without hashing it — which is what makes re-importing a
    /// 2000-frame folder take seconds instead of minutes.
    pub mtime_ms: i64,
    pub kind: PhotoKind,
    /// Pixel dimensions *after* EXIF orientation is applied, so a portrait frame
    /// off a camera that stored it sideways reads taller than it is wide. `None`
    /// when nothing could decode the file — a RAW without a decoder, or a
    /// corrupt frame that was catalogued anyway.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Raw EXIF orientation tag, 1..=8, kept as the camera wrote it.
    ///
    /// `width`/`height` above already account for it; this is retained so a
    /// decoder can re-apply the transform to the original pixels. Do not use it
    /// to decide how to turn a *developed* image — develop's own geometry ops
    /// are the authority there.
    pub orientation: Option<u16>,
    /// ISO-8601 capture time from EXIF, when present.
    ///
    /// No timezone: EXIF does not record one, so this is local time at the
    /// camera and is only ever compared as a string. Absent for scans, screen
    /// grabs and anything stripped of metadata — and those sort last rather than
    /// first, deliberately, in every captured-date order.
    pub captured_at: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    /// Lens as the body named it (`LensModel`). Free text from the file, so it
    /// can be anything — including markup, on a frame from a client.
    pub lens: Option<String>,
    /// Sensitivity, e.g. `1600`.
    pub iso: Option<i64>,
    /// f-number — `2.8`, not `"f/2.8"`.
    pub aperture: Option<f64>,
    /// Exposure time in **seconds**, so a 1/250 s frame is `0.004`. Displaying
    /// it as a fraction is the caller's job.
    pub shutter: Option<f64>,
    /// Focal length in millimetres, as recorded — the physical length, not a
    /// 35 mm equivalent.
    pub focal_length: Option<f64>,
    /// Stars, 0..=5. Zero means unrated; the setters clamp anything above five
    /// rather than refuse it.
    pub rating: u8,
    pub flag: Flag,
    /// Free-form colour label. A string rather than an enum because the set is
    /// the photographer's, not ours, and an unknown one from another tool's
    /// catalog should survive a round trip instead of being flattened.
    pub color_label: Option<String>,
    /// Tiny base64 data URI used as a blur placeholder by the web gallery.
    pub blur_lqip: Option<String>,
    /// RFC-3339 timestamp of the *first* import of this path. Re-importing a
    /// changed file updates every other field and leaves this one, so it dates
    /// when the photograph entered the library, not when it was last touched.
    pub imported_at: String,
}

impl Photo {
    /// Camera make + model as one display string, matching what the web
    /// gallery's search indexes.
    pub fn camera(&self) -> Option<String> {
        let combined = [self.camera_make.as_deref(), self.camera_model.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let trimmed = combined.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }
}

/// Query filter for the photo grid. All conditions are ANDed; `None` means
/// "don't constrain on this".
///
/// The field names cross an IPC boundary from JavaScript, so they are named
/// the way JavaScript names things. Without that, serde quietly dropped
/// `minRating` and `albumPath` as unknown fields and both the star filter and
/// the album sidebar looked like they worked while filtering nothing.
/// `deny_unknown_fields` is what makes the next such typo an error rather than
/// a feature that silently does nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhotoFilter {
    /// Minimum star rating (inclusive).
    pub min_rating: Option<u8>,
    /// Exact flag match, [`Flag::None`] included — asking for `None` selects the
    /// untriaged frames rather than dropping the condition. Dropping it is what
    /// `None` on this `Option` means.
    pub flag: Option<Flag>,
    /// Exact colour label. Compared as written, case included, since the labels
    /// are the photographer's own strings.
    pub color_label: Option<String>,
    pub kind: Option<PhotoKind>,
    /// Exact camera model — one of the strings [`Library::camera_models`] hands
    /// the filter UI, not a substring. Use [`text`](Self::text) for a substring.
    ///
    /// [`Library::camera_models`]: crate::Library::camera_models
    pub camera_model: Option<String>,
    /// Free text over filename and camera.
    ///
    /// Case-insensitive substring, with SQL's `%` and `_` escaped — a search for
    /// `DSC_0042` finds that frame and not the Fuji's `DSCF0042`.
    pub text: Option<String>,
    /// Restrict to members of this album path.
    ///
    /// Membership is direct, not inherited: a collection's path matches nothing,
    /// because photos hang off albums and never off the folders above them.
    pub album_path: Option<String>,
    /// Restrict to photos carrying this tag, exactly as stored. Tags are
    /// written trimmed and lowercased (see `Library::set_photo_tags`), so a
    /// caller should ask in lowercase too.
    pub tag: Option<String>,
    /// ISO date bounds on `captured_at` (inclusive), e.g. "2026-06-01".
    ///
    /// Compared as strings against the stored ISO timestamp, which works only
    /// because both are ISO-8601. A frame with no `captured_at` fails both
    /// bounds, so setting either one silently excludes the undated photographs.
    pub captured_from: Option<String>,
    /// Upper bound, widened to the end of that day before it hits SQL, so
    /// `"2026-06-14"` includes a frame shot at 23:59 rather than only midnight.
    pub captured_to: Option<String>,
    /// Row cap. `None` returns the whole library — fine for a catalog of
    /// thousands, less so for one of hundreds of thousands.
    pub limit: Option<u32>,
    /// Rows to skip. **Ignored unless `limit` is also set**, because SQLite has
    /// no `OFFSET` without a `LIMIT`; a paginating caller must send both.
    pub offset: Option<u32>,
    /// Defaulted, unlike the `Option` fields above, because omitting a sort
    /// means "the usual one" rather than "do not sort". Without this the
    /// struct could not be deserialized from `{}` even though it derives
    /// `Default` — a trap the web UI never hit only because it always sends a
    /// sort, and one the first caller writing JSON by hand walks straight into.
    #[serde(default)]
    pub sort: PhotoSort,
}

/// Grid ordering, as the sort dropdown offers it.
///
/// Every variant breaks its own ties on filename ascending, so a burst of frames
/// with one capture second between them, or a hundred photographs all rated
/// three stars, come back in a stable order — the grid must not reshuffle
/// underneath a photographer scrolling it.
///
/// Serialised kebab-case (`"captured-asc"`), which is what the UI and any FFI
/// caller send.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhotoSort {
    /// Oldest first, and the default: a shoot read in the order it happened.
    /// Frames with no `captured_at` sort to the **end** here, not the beginning,
    /// so an undated scan does not open the wedding.
    #[default]
    CapturedAsc,
    /// Newest first — undated frames still last, same rule.
    CapturedDesc,
    NameAsc,
    NameDesc,
    /// Five stars first. Unrated frames are zero, so they land at the bottom.
    RatingDesc,
    /// Position within an album; only meaningful with `album_path` set.
    ///
    /// Without an album there is no position column in the query, so this
    /// quietly degrades to `NameAsc` rather than failing — see the test in
    /// `catalog`. This is the order publishing writes as `photoOrder`.
    AlbumOrder,
}

/// Album settings — mirrors the frontmatter schema the Astro site reads.
/// The mapping is asserted by a test in `publish`.
///
/// This is the whole of what publishing writes into an album's `index.md`, so a
/// field here is a field on the live gallery: the two that gate access
/// ([`password`](Self::password), [`share_token`](Self::share_token)) and the
/// two that let visitors take things away
/// ([`allow_download`](Self::allow_download), [`proofing`](Self::proofing)) are
/// worth reading twice before changing in bulk. Several are plain `String`
/// rather than enums because the vocabulary belongs to the Astro site, which
/// ships on its own schedule; an unrecognised value must survive a round trip
/// through this catalog rather than be normalised away.
///
/// Rows come back from the catalog only partly filled — `cover_filename` and
/// `tags` live in other tables and are hydrated afterwards, so an `Album` built
/// by hand rather than read through [`Library`](crate::Library) is missing them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: i64,
    /// e.g. "2026/weddings/ana-ivan"
    ///
    /// Both the identity of the album and its folder on disk, in the published
    /// tree and on the server — which is why renaming one is a move, not an
    /// update, and why the sync subscription has to be carried across with it.
    pub path: String,
    /// `path` minus its last segment; `None` for a top-level album. The gallery
    /// navigates by this, and access inheritance walks up it, so every ancestor
    /// named here must exist as a collection row.
    pub parent_path: Option<String>,
    /// Display name, published as `title`. Derived from the folder name when an
    /// album is created without one — but only then: an update may write an
    /// empty string, and the gallery will show a nameless album rather than
    /// fall back.
    pub title: String,
    pub description: Option<String>,
    /// ISO date (yyyy-mm-dd).
    ///
    /// The date of the *shoot*, typed by the photographer — unrelated to any
    /// frame's EXIF, and not used for sorting anything.
    pub date: Option<String>,
    /// Internal id stored in the site's access cookie. Grants nothing alone.
    pub token: String,
    /// Plaintext, and published as plaintext into `index.md`. That is the web
    /// gallery's own scheme, matched deliberately; it keeps a client out of a
    /// preview gallery and is not a secret worth anything against someone who
    /// can read the server's files.
    pub password: Option<String>,
    /// Random secret enabling the "secret link" access mode.
    ///
    /// Unlike `token` this *is* a credential — anyone holding it opens the album
    /// without the password. Generated by
    /// [`albums::generate_share_token`](crate::albums::generate_share_token);
    /// never type one by hand.
    pub share_token: Option<String>,
    /// Photo order the site applies, e.g. `"date-desc"` (the default) or
    /// `"custom"`. `"custom"` is the one with a consequence here: it makes
    /// publishing emit the explicit `photoOrder` list, and without it a
    /// drag-and-drop arrangement is written to the catalog and never reaches the
    /// gallery.
    pub sort: String,
    /// Layout the site renders, e.g. `"single-column"` (the default), `"grid"`,
    /// `"masonry"`, `"slideshow"`.
    pub style: String,
    /// Cover photo's filename, hydrated from the `cover_photo_id` the row
    /// actually stores. `None` means no cover was chosen — or the chosen photo
    /// has since left the catalog — and the gallery then falls back to the first
    /// photo. Publishing drops it too if it does not match a photo in the album,
    /// rather than writing a `thumbnail:` pointing at nothing.
    pub cover_filename: Option<String>,
    /// A folder of albums rather than a folder of photos. Collections hold no
    /// photos — adding some is refused — and the gallery draws their sub-albums.
    pub is_collection: bool,
    /// Kept out of listings. Still reachable by its URL and still published, so
    /// this hides an album, it does not protect one; that is `password` and
    /// `share_token`.
    pub hidden: bool,
    /// Offer the ZIP download of the whole album. Off by default: an album is
    /// not downloadable unless someone said so.
    pub allow_download: bool,
    /// Turn on client proofing — hearts, comments, and a submission the gallery
    /// writes into the album's server-owned `.meta/`. Sync never touches that
    /// folder, so a photographer's push cannot delete a client's selections.
    pub proofing: bool,
    /// Manual position among siblings, published as `order`. Lower first;
    /// `None` sorts after everything numbered.
    pub sort_order: Option<i64>,
    /// Album tags, hydrated from the join table. Each becomes a pill on the
    /// album page linking to a prerendered `/photos/tags/<tag>` page — so a tag
    /// on a locked album still names it in public.
    pub tags: Vec<String>,
    /// Markdown prose, published beside the frontmatter as `body.md`. Clearing
    /// it deletes that file on the next publish.
    pub body: Option<String>,
}

impl Album {
    /// An album is "locked" when it has a password and/or a share token —
    /// the same rule the web gallery applies.
    pub fn is_locked(&self) -> bool {
        self.password.is_some() || self.share_token.is_some()
    }

    /// Last path segment, used as the folder name.
    pub fn slug(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// Progress callback payload for long-running imports.
///
/// Delivered from the worker threads, so it arrives **out of order and from more
/// than one thread at once**: `processed` is a running count of files started,
/// not an index, and `current` is whichever file that particular worker picked
/// up. A caller that renders it must be cheap and must not assume the counter
/// only ever moves forward by one.
///
/// An import that has to copy a folder into the library reports that copy as its
/// own pass first, with its own `total`, so the bar reaches the end twice.
#[derive(Debug, Clone, Serialize)]
pub struct ImportProgress {
    pub processed: usize,
    /// Files in this pass. Fixed before work starts, so the bar does not stretch
    /// mid-import.
    pub total: usize,
    /// The file this tick is about — library-relative during the scan, prefixed
    /// `copying ` during the copy-in pass. For display only; nothing parses it.
    pub current: String,
}

/// Summary returned by an import run.
///
/// **The counts overlap and do not add up to the number of files seen.** A file
/// catalogued for the first time whose bytes already exist elsewhere in the
/// library counts in *both* `imported` and `duplicates`; an `undecodable` frame
/// was still imported. Only `imported`, `updated`, `skipped` and `failed` are
/// disjoint. Presenting `imported + duplicates` as a total overstates the run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportSummary {
    /// New catalog rows — every file the library did not already have at that
    /// path, duplicates of other paths included.
    pub imported: usize,
    /// Files already in the catalog with the same content hash.
    ///
    /// The same photograph reached the library twice under two names — the usual
    /// cause is a card offloaded once by hand and once by the app. Nothing is
    /// deduplicated; both rows stay, and this is the count that lets the caller
    /// say so.
    pub duplicates: usize,
    /// Files whose content changed since last import.
    ///
    /// The path was known, the bytes were not: an existing row was overwritten
    /// with fresh metadata. Note that this counts a *replaced* photograph as
    /// readily as a re-saved one, and the row's `imported_at` still dates the
    /// original.
    pub updated: usize,
    /// Unchanged since the last run — same size, same mtime — so nothing was
    /// read, hashed or decoded. On a re-import of a settled library this is
    /// nearly everything, and that is the point.
    pub skipped: usize,
    /// `(library-relative path, message)` for each file that could not be
    /// processed at all. One failure never stops the run: the other frames are
    /// imported and named failures ride out here, because a folder arriving
    /// three photographs short with no complaint looks exactly like a clean
    /// import.
    pub failed: Vec<(String, String)>,
    /// Files copied into the library because they came from outside it — a
    /// camera card, a download folder. Zero when importing in place.
    pub copied_in: usize,
    /// Where those copies landed, relative to the library root.
    pub copied_into: Option<String>,
    /// Catalogued, but no decoder could read the pixels — a corrupt file, or a
    /// RAW format this build does not develop. They are kept rather than
    /// dropped, because a file on disk that the catalog forgets is worse than
    /// one it cannot preview, but the caller should say so.
    pub undecodable: Vec<String>,
    /// The run stopped on request before reaching the end of the folder. Every
    /// other count is then a partial tally, so a caller that ignores this flag
    /// would announce a finished import that never finished.
    pub cancelled: bool,
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    /// The desktop UI builds this object in JavaScript. Every field it sends
    /// has to land: an unrecognised key used to be dropped in silence, which
    /// is how the star filter and the album sidebar came to filter nothing.
    #[test]
    fn photo_filter_accepts_what_the_ui_sends() {
        let json = r#"{
            "minRating": 3,
            "flag": "pick",
            "text": "nikon",
            "albumPath": "2026/weddings/ana-ivan",
            "sort": "album-order",
            "limit": 2000
        }"#;
        let filter: PhotoFilter = serde_json::from_str(json).unwrap();
        assert_eq!(filter.min_rating, Some(3));
        assert_eq!(filter.flag, Some(Flag::Pick));
        assert_eq!(filter.album_path.as_deref(), Some("2026/weddings/ana-ivan"));
        assert_eq!(filter.text.as_deref(), Some("nikon"));
        assert_eq!(filter.limit, Some(2000));
    }

    /// A filter that constrains nothing is the commonest one there is, so the
    /// empty object has to mean it. Whoever writes the JSON by hand — a CLI, a
    /// Swift client over the FFI — starts from `{}` and adds keys.
    #[test]
    fn photo_filter_reads_back_from_an_empty_object() {
        let filter: PhotoFilter = serde_json::from_str("{}").unwrap();
        assert!(filter.min_rating.is_none());
        assert_eq!(filter.sort, PhotoSort::default());
    }

    /// And a name that is not a field is a loud error, not a no-op.
    #[test]
    fn photo_filter_rejects_an_unknown_field() {
        let err = serde_json::from_str::<PhotoFilter>(r#"{"min_rating": 3}"#).unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}
