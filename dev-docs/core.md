# gpp-core — catalog, import, develop, publish

The Rust core holding all desktop logic. No GUI, no async runtime, no spawned
processes; everything platform-specific behind a trait. This document covers
the library-local half; [sync.md](sync.md) covers everything that talks to a
server.

> Historical note: an earlier version of this document lived at
> `desktop/ARCHITECTURE.md` and described develop and RAW as "out of scope,
> phase B". Develop shipped; this document describes what exists. The
> `RawDecoder` trait still awaits its LibRaw implementation (CDDL-1.0 route,
> lawyer's review before commercial shipping — see the trait's docs).

## The library on disk

A library is a folder of photos the user already has. The app never moves
them; it writes a catalog beside them:

```
<library>/
  2026/weddings/ana-ivan/*.jpg     the photographer's own folders, untouched
  .gpp/catalog.db                  SQLite: photos, albums, edits, sync state
  .gpp/thumbs/<shard>/<key>_*.jpg  content-addressed derived images
```

Importing a folder from **outside** the library copies it in
(`import::bring_inside`), because the catalog addresses photos by their path
under the root and cannot point anywhere else. The CLI and the app both say
where the copies landed — a library silently growing by a card's worth of
gigabytes is not acceptable.

**The catalog is a derived index.** The filesystem holds the pixels; a lost or
corrupt catalog is rebuilt by re-importing. That is why a corrupt DB is an
inconvenience and never data loss — and why nothing is allowed to make the
catalog the only holder of a fact about pixels.

## Catalog (SQLite)

WAL mode, foreign keys on, `bundled` rusqlite (no system SQLite anywhere), an
explicit busy timeout so a second writer waits instead of failing. Schema
migrations run **in one transaction** with idempotent DDL — a process killed
mid-migration used to leave the library permanently unopenable, and the module
doc's promise that "a failed migration is recoverable" is only true if the
database still opens. A catalog whose stored version is *newer* than the build
refuses to open, naming both versions; an old build silently running against a
future schema is how data gets mangled quietly.

Tables in brief: `photos` (identity = `content_hash`, blake3), `albums`
(identity = `path`), `album_photos` (membership + position), `edits` (one JSON
op stack per photo), `album_sync` + `sync_state` + `published_files` (see
[sync.md](sync.md)), `settings`, `schema_version`.

## Import

`import_dir` scans (extension-filtered — see below), hashes, reads EXIF,
generates thumbnails, all in parallel over rayon. Three behaviours that were
each a shipped defect before they were rules:

- **Cancellable, and a stopped run commits its prefix.** The stop flag is
  polled before every file in both passes. The photographer who stops at 600
  keeps those 600; `ImportSummary::cancelled` prevents a partial run being
  reported as a complete one. `Session::cancel_import` is non-blocking and
  callable from another thread — which is the whole point, since `import`
  blocks the thread it runs on.
- **Metadata-only imports read headers, not pixels.** With thumbnails off, the
  dimensions come from the file header (`media::read_dimensions`) — reading
  24 megapixels to learn two numbers took a 1.1 GB pass from 0.3 s to 136 s.
  HEIF is the exception: it has no cheap header path and costs a decode.
- **Undecodable files are catalogued and flagged**, never skipped silently. A
  file on disk the catalog has forgotten is worse than one it cannot preview.
  Publish then excludes them (`unrenderable`), so they never reach the site as
  broken images.

**Format support** is `media::classify` over three extension lists.
`IMAGE_EXTENSIONS` promises `media::decode` can open the file — the `image`
crate for JPEG/PNG/WebP/TIFF/GIF, a pure-Rust HEVC decoder (`heif-oxide`) for
HEIC/HEIF. RAW extensions are catalogued (metadata + embedded preview when
present) but never published — a RAW is a negative, not something to hand a
client. Nothing sniffs magic bytes; a mislabelled file fails at decode and is
flagged.

HEIF numbers, measured: ~9.4 MP/s on one core, so ~2.6 s for a 24 MP frame
against a few hundred milliseconds for JPEG. Import parallelises across cores,
so a card is minutes — but a HEIC import is visibly slower than a JPEG one and
that is why.

## Develop — non-destructive, and what makes it cheap

An adjustment is a row in `edits`, never a write to the original. What
identifies a rendered image is the **render key**:

```
untouched photo:  key = content_hash                  (the original IS the render)
adjusted photo:   key = blake3(content_hash ⊕ stack_json ⊕ encoder_quality)
```

Everything follows from that one idea:

| Property | Because |
|---|---|
| Adding develop invalidated nothing | empty stack ⇒ the content hash, unchanged |
| A stale render can never be served | different edits (or a different encoder) are a different key |
| Undo is instant | the previous key is probably still cached |
| Reset costs nothing | `ensure_rendered` returns the original's own path for an empty stack |
| Publish never re-develops | it copies the cached full-size render |

The encoder quality is in the key because it is part of what the pixels *are*:
when `DELIVERY_JPEG_QUALITY` moved from the image crate's default (75) to 92,
libraries with existing renders would otherwise have published the old bytes
forever while new photos got the new ones. Thumbnails deliberately stay at 75 —
they are never delivered to anyone and are redrawn constantly while culling.

### Orientation is canonical — do not edit ops in place

Turns and mirrors do not commute. Stored orientation is **one left-to-right
mirror followed by quarter turns** — the eight ways a rectangle can be set
down. `flip-vertical` is *never written* (it is a mirror plus a half turn).
Every button, and the generic `set(EditOp)` the C ABI exposes, composes onto
the *outside* of the current framing and writes the whole thing back.

This is the fix for two defects that were three presses away in the shipping
panel: "rotate right" on a flipped frame turned the photograph left, and
pressing a flip again to undo it mirrored the wrong axis once a turn sat
between them. Two case-by-case patches failed before the canonical form; the
680-case matrix in `tests/geometry_order.rs` is what holds it now.

Consequences for anyone touching this code:

- **Anything reading orientation must fold the whole op list** (as the UI's
  `renderGeometry` does), never look for a particular op — asking "is there a
  flip-vertical?" answers no about a photograph that is plainly upside down.
- **The crop is held last among geometry** and its rectangle is carried
  through every turn and mirror, so a frame drawn on a face keeps framing the
  face after the shot is straightened. Legacy stacks written before this rule
  (crop recorded mid-stack) are normalised on the next geometry press, with
  the rectangle carried through the framing that used to follow it.
- `apply` runs two passes: geometry in stack order, then tone in stack order,
  purely per pixel. A tone op's position relative to geometry is therefore
  irrelevant — measured, not assumed. Within tone, order matters, as in any
  developer.

## Publish

Publishing materialises the gallery's content tree for one album: `index.md`
frontmatter generated from the album row, optional `body.md`, and the
**developed** photo files. Rules that each earned their place:

- **Prune only what this library published.** `published_files` records every
  file this machine put in the tree; only those are prune candidates. The
  admin panel writes into the same folders, and its files are never ours to
  remove. Dotfiles and subdirectories are not even read.
- **`published_filename` is the single answer to "what is this called on the
  site".** Used by the copy, the skip check, `photoOrder`, prune and collision
  detection alike. HEIF publishes as `.jpg` (Chrome and Firefox cannot display
  HEIC; a developed HEIC is JPEG bytes anyway), and converting *either way*
  keeps the name — the published filename is the URL a client may already
  hold, and moving a slider must not move their photograph.
- **Same-named frames are reported, never silently renamed.** Two cards both
  produce `DSC_0001.jpg`; the first in album order keeps the name, the rest
  are named in `PublishResult.collisions` and ship nothing. Renaming would
  change URLs behind the client's back.
- **One bad frame never stops the album.** Missing originals land in
  `missing`, undecodable ones in `unrenderable`, and the other four hundred
  photographs still reach the client.
- `photoOrder` lists every published photo, so the site's partial-order
  fallback never fires; `reorder_album` keeps unlisted photos *after* the
  listed ones in their existing relative order, matching what the site does.

## The C ABI (`gpp-ffi`)

Four `extern "C"` functions; the whole `Session` surface goes through
`gpp_call(session, method, args_json)` so forty-odd hand-written C signatures
cannot drift on the first added field. Replies are always `{"ok": …}` or
`{"error": …, "kind": …}`; every entry point catches unwinding, because a
panic reaching C is undefined behaviour. Argument keys are `Session`'s
parameter names unchanged (snake_case); the only camelCase is inside the typed
structs the core itself owns (`PhotoFilter`, `NewAlbum`, `AlbumUpdate`), and
`deny_unknown_fields` turns a typo into an error instead of a silently dropped
field. The header is hand-written and checked in (`include/gpp_ffi.h`), with a
test asserting it matches the code — cbindgen and UniFFI are MPL-2.0, which
the licence policy does not allow even at build time.

`remote_token` is deliberately not exposed; `has_remote_token` answers the
only question a UI has without handing the secret to every linked caller.

## The shell (`desktop/app`)

Tauri v2; one `#[tauri::command]` per UI action, each a one-liner into
`Session`. The UI is embedded into the binary at compile time — **rebuild
after every UI edit or you are testing the previous JavaScript** (`build.rs`
watches the UI files; `check-shell.mjs` derives the watch list from the
directory so a new file cannot be forgotten). The shell cannot be unit-tested,
so it has contract checks instead — see [testing.md](testing.md), including
the two webview traps (a `confirm()` that answers itself; serde silently
dropping unknown field names) that each shipped a defect before becoming a
check.
