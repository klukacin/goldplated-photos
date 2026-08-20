# Goldplated Photos — Desktop Architecture

> Native photo management + publishing app. Rust core, Tauri shell.
> Target platforms in order: **macOS → iPadOS → Windows/Linux**.
> Long-term direction: a focused "Lightroom CC"-style tool for photographers
> whose deliverable is a published client gallery.

---

## 1. Scope of this phase

**In scope (phase A — functional macOS app):**

| Capability | Notes |
|---|---|
| Import | Folder scan, dedup by content hash, metadata extraction, thumbnail generation |
| Rating | 0–5 stars, pick/reject flag, color labels, fast filtering |
| Album management | Hierarchical albums, add/remove photos, custom order, all gallery settings |
| Push to web | Materialize the Astro content tree, sync to the server without blind deletes |
| Publish | Set access (password / share link), proofing, download — then push |

**Explicitly out of scope for phase A** (kept possible by the architecture, not built yet):
RAW decoding, non-destructive develop pipeline, local adjustments, mobile app.
The catalog schema and the `RawDecoder` trait exist so these bolt on without a rewrite.

---

## 2. Why this shape

Three constraints drive every decision below.

**The core must run on iPadOS.** That rules out desktop-only assumptions in the
core: no `std::process::Command`, no shelling out to `rsync`, no absolute-path
assumptions, no threads-that-outlive-the-app. Everything platform-specific
lives behind a trait, implemented per platform in the shell.

**The web gallery stays as-is.** The Astro site is the delivery surface and it
already has production-grade access control (HMAC-signed cookies, opt-in share
tokens, per-route enforcement). The desktop app *feeds* it; it does not replace
it. The published artifact is exactly the `src/content/albums/**` tree the site
already reads, so nothing downstream changes.

**Sync must never destroy data.** The current `rsync --delete` model cannot tell
"deleted on purpose" from "this machine never had it". The core keeps a
three-way sync state so it can.

---

## 3. Workspace layout

```
desktop/
├── ARCHITECTURE.md          ← this file
├── Cargo.toml               ← workspace root
├── crates/
│   ├── gpp-core/            ← all logic; pure Rust, no GUI, iOS-safe
│   │   ├── catalog/         ← SQLite: schema, migrations, queries
│   │   ├── import/          ← scan, hash, metadata, thumbnails
│   │   ├── media/           ← decode/resize abstraction, RawDecoder trait
│   │   ├── albums/          ← album tree, membership, ordering
│   │   ├── publish/         ← materialize Astro content tree
│   │   └── sync/            ← three-way state, transfer trait
│   └── gpp-cli/             ← thin CLI over the core (testing + automation)
└── app/                     ← Tauri v2 shell (macOS first, iOS target ready)
    ├── src-tauri/           ← commands → gpp-core
    └── ui/                  ← web UI (reuses admin patterns)
```

**Rule: `gpp-core` never depends on Tauri, and the UI never touches SQLite.**
Every capability is a core function first and a Tauri command second. That is
what makes the iPad build a packaging exercise rather than a port.

---

## 4. Catalog (SQLite)

One database per **library**, stored at `<library>/.gpp/catalog.db`. The library
root is a folder the user picks; photos live under it.

WAL mode, `foreign_keys=ON`, single writer + many readers. `rusqlite` with the
`bundled` feature so there is no system SQLite dependency on any platform.

### Tables

```sql
photos(
  id INTEGER PRIMARY KEY,
  rel_path TEXT NOT NULL UNIQUE,   -- relative to library root, '/' separated
  filename TEXT NOT NULL,
  content_hash TEXT NOT NULL,      -- blake3, dedup + sync identity
  file_size INTEGER NOT NULL,
  mtime_ms INTEGER NOT NULL,       -- cheap invalidation
  kind TEXT NOT NULL,              -- 'photo' | 'video' | 'raw'
  width INTEGER, height INTEGER,
  orientation INTEGER,
  captured_at TEXT,                -- ISO 8601, from EXIF
  camera_make TEXT, camera_model TEXT, lens TEXT,
  iso INTEGER, aperture REAL, shutter REAL, focal_length REAL,
  rating INTEGER NOT NULL DEFAULT 0,        -- 0..5
  flag TEXT NOT NULL DEFAULT 'none',        -- 'none' | 'pick' | 'reject'
  color_label TEXT,                         -- red/yellow/green/blue/purple
  blur_lqip TEXT,                           -- base64 data URI for the web grid
  imported_at TEXT NOT NULL
)

albums(
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,       -- '2026/weddings/ana-ivan'
  parent_path TEXT,
  title TEXT NOT NULL,
  description TEXT, date TEXT,
  token TEXT NOT NULL,             -- internal id for the access cookie
  password TEXT, share_token TEXT, -- access model (see §7)
  sort TEXT NOT NULL DEFAULT 'date-desc',
  style TEXT NOT NULL DEFAULT 'single-column',
  cover_photo_id INTEGER REFERENCES photos(id) ON DELETE SET NULL,
  is_collection INTEGER NOT NULL DEFAULT 0,
  hidden INTEGER NOT NULL DEFAULT 0,
  allow_download INTEGER NOT NULL DEFAULT 0,
  proofing INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER,
  body TEXT
)

album_photos(album_id, photo_id, position)   -- PK(album_id, photo_id)
tags(id, name UNIQUE)
photo_tags(photo_id, tag_id)
album_tags(album_id, tag_id)

edits(                              -- reserved for the develop phase
  photo_id INTEGER PRIMARY KEY,
  version INTEGER NOT NULL,
  stack_json TEXT NOT NULL          -- ordered, non-destructive operation list
)

sync_state(                         -- see §6
  entity_kind TEXT, entity_key TEXT,
  local_hash TEXT, synced_hash TEXT, remote_hash TEXT,
  last_synced_at TEXT,
  PRIMARY KEY(entity_kind, entity_key)
)

settings(key TEXT PRIMARY KEY, value TEXT)
schema_version(version INTEGER)
```

### Why a DB at all

The web app scans directories and caches JSON per album. That is fine for one
album at a time and hopeless for a 50,000-photo library: filtering by "rating ≥ 4
and shot in June on the R5" must be a single indexed query, not a filesystem
walk. Indices on `rating`, `captured_at`, `camera_model`, `content_hash`.

**The filesystem remains the source of truth for pixels; the DB is a derived
index.** A lost catalog is rebuildable by re-importing. That is deliberate — it
means a corrupt DB is an inconvenience, never data loss.

---

## 5. Media pipeline

```
                        ┌──────────────────┐
   file  ──────────────►│  MediaDecoder    │──► RgbImage + Metadata
                        └──────────────────┘
                          ▲              ▲
                 ImageCrate            RawDecoder (trait)
                 (jpeg/png/tiff/webp)   └── LibRawDecoder  (phase B, CDDL)
                                        └── NullRawDecoder (phase A: skip RAW)
```

`RawDecoder` is a trait from day one:

```rust
pub trait RawDecoder: Send + Sync {
    fn supports(&self, ext: &str) -> bool;
    fn decode(&self, path: &Path) -> Result<DecodedRaw>;
    fn embedded_preview(&self, path: &Path) -> Result<Option<Vec<u8>>>;
}
```

Phase A ships `NullRawDecoder` (RAW files are catalogued with metadata but use
the embedded JPEG preview when available). Phase B swaps in a LibRaw-backed
implementation. **Nothing else in the codebase changes** — that is the point of
the trait.

**Licensing note.** LibRaw is dual-licensed LGPL-2.1 **or CDDL-1.0**. CDDL is
file-level copyleft and permits static linking into a proprietary product: only
modifications to LibRaw's own files must be published. That is the intended
route for a commercial build. Binding crates carry their own licenses — write a
thin `bindgen` wrapper rather than adopting an LGPL binding crate. Get a lawyer's
read before shipping commercially; this note is engineering guidance, not legal
advice.

**Thumbnails.** Three sizes (400 / 1200 / 1920 px long edge) plus a ~20 px blur
LQIP, matching what the web gallery already serves. `fast_image_resize` (SIMD)
for the resize, written to `<library>/.gpp/thumbs/<hash[0:2]>/<hash>_<size>.jpg`.
Content-addressed, so re-importing the same file is free and moving a file does
not orphan its thumbnails.

**Parallelism.** `rayon` over the import set, bounded by available cores. This
is the single biggest speed win over the Node admin: real threads, no event loop,
no GC pause.

---

## 6. Sync — the part that must not lose data

The failure mode we are designing away: machine B holds a subset of the library,
runs a publish, and the server loses everything B does not have.

Three states per entity: `local_hash`, `synced_hash` (what we last agreed on with
the server), `remote_hash` (what the server reports now).

| local | synced | remote | Meaning | Action |
|---|---|---|---|---|
| A | A | A | in sync | nothing |
| B | A | A | changed locally | push |
| A | A | B | changed remotely | pull |
| B | A | C | changed on both | **conflict → ask** |
| — | A | A | deleted locally, on purpose | delete remote (confirm) |
| — | — | A | never had it on this machine | **leave alone** |
| A | — | — | new locally | push |

The last two rows are exactly the distinction `rsync --delete` cannot make, and
the reason this table exists.

Transfer is behind a trait so the core stays iOS-safe (no shelling out to
`rsync`):

```rust
pub trait RemoteTransport: Send + Sync {
    fn manifest(&self) -> Result<RemoteManifest>;         // hashes from server
    fn put(&self, rel: &str, bytes: &[u8]) -> Result<()>;
    fn get(&self, rel: &str) -> Result<Vec<u8>>;
    fn delete(&self, rel: &str) -> Result<()>;
}
```

Implementations: `SftpTransport` (desktop, matches today's server), later
`HttpTransport` (works on iPad, where SSH is impractical). Server-side manifest
generation is a small endpoint or a one-line `find`+hash over SSH.

**Server-owned data is never pushed over.** Proofing submissions live in
`<album>/.meta/proofing/` and are written by clients on the server. They are
pull-only. (This is a live bug in the current bash deploy — every deploy deletes
them — and the new engine fixes it by construction.)

---

## 7. Publish — what actually lands on the server

Publishing materializes the exact tree the Astro site already reads:

```
src/content/albums/<album-path>/
├── index.md          ← frontmatter generated from the albums table
├── body.md           ← optional prose
├── <selected>.jpg    ← exported photos
└── .meta/            ← thumbs + proofing (server-owned, pull-only)
```

Frontmatter fields map 1:1 to the album row and to the site's content schema:
`title, description, date, token, password, shareToken, sort, photoOrder, style,
thumbnail, tags, isCollection, order, hidden, allowDownload, proofing`.

That mapping is the contract between the two codebases. It is asserted by a test
in the core so a schema drift fails CI rather than silently producing an album
the site refuses to render.

**Access control is set here, not on the server.** Password and share token are
album fields; the app generates share tokens with a CSPRNG (matching the site's
`shareToken` semantics) and shows the shareable link.

---

## 8. Desktop shell (Tauri v2)

Tauri over Electron: ~10 MB binary instead of ~150 MB, system webview, and — the
deciding factor — **Tauri v2 targets iOS**, so the same Rust core drives the iPad
app later.

The UI is web tech, which lets us reuse the interaction patterns already built in
the web admin (album tree, grid, drag-reorder, bulk select). Commands are thin:

```
import_folder(path)        rate_photo(id, rating)      set_flag(id, flag)
list_photos(filter)        create_album(...)           add_to_album(...)
reorder_album(...)         publish_album(path)         sync_status()
```

Each is a one-liner into `gpp-core`. No logic in the shell.

**macOS specifics:** signing + notarization needed for distribution outside the
App Store (Apple Developer account). Unsigned local builds run fine for
development. Security-scoped bookmarks are required to retain folder access
across launches once sandboxed — noted now because retrofitting it is painful.

**iPad path:** the same core compiles for `aarch64-apple-ios`. What changes is
the shell: file access via document picker rather than arbitrary paths, and
`HttpTransport` instead of SFTP. Both are already behind traits.

---

## 9. Build & verification status

The Rust core builds and its tests run **on any platform, including this Linux
dev container** — that is the point of keeping GUI out of the core.

The macOS `.app` bundle can only be produced on a Mac (Xcode toolchain). The
Tauri shell in `app/` is written and structured here; building and running it is
a `cargo tauri dev` on the target machine. Anything I cannot execute here is
called out explicitly rather than reported as verified.

---

## 10. Sequencing

1. **Core: catalog + import + rating + albums** — the foundation, fully testable
2. **Core: publish + sync** — solves the multi-machine problem that started this
3. **CLI** — drives the core headlessly; also a bridge for the existing web admin
4. **Tauri macOS app** — functional app; end of this phase
5. *(next phase)* RAW via LibRaw/CDDL, develop pipeline, iPad shell
