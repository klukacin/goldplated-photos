# Multi-library, multi-remote, master catalog — design decisions

> Status: **agreed direction, phased delivery** — defined 2026-08-24 with the
> project owner. Phase 1 (Lightroom import + library switching) and phase 2
> (multiple remotes, sync scopes, cross-library push) are being implemented;
> the master catalog is a design constraint for now, not a deliverable.
> This document records the decisions so later schema work does not
> accidentally close the door on them.

## Vocabulary

- **Library** — a folder of photos with a `.gpp/catalog.db` beside them.
  A machine can hold several; the app knows them all (a small registry in
  the app config dir) and switches between them. Exactly one is open at a
  time in the core (`Session` = one library).
- **Remote** — a sync destination a library knows: a folder or an HTTP
  server speaking `/api/sync/*`. Credentials live **in the library's
  catalog** (they belong to the library and travel with the drive), but the
  app-level registry may enumerate every known library's remotes so the UI
  can offer "push to <remote> (from library X)" across libraries.
- **Main remote** — each library's default remote (today's single
  `remote.dir`/`remote.token` pair becomes remote #1 after migration).
  It is the device-to-device sync channel for that library.
- **Publish target** — a local rendered-tree destination (`dest_root`).
  Distinct from a remote, as today: publish writes the tree, sync moves it.

## Sync scopes: `web` vs `full`

A subscription (album ↔ remote) carries a **scope**:

- **`web`** (today's behavior, stays the default): what syncs is the
  *published* tree — developed pixels, JPEG for HEIF, RAW excluded,
  `index.md` frontmatter. This is "what galleries show".
- **`full`**: additionally syncs the *originals* — every catalogued file of
  the album's photos, byte-identical, plus a per-album metadata sidecar
  (see XMP below) carrying ratings/flags/labels/keywords and the develop
  stacks. This is "everything, with all details", and it is what makes the
  main remote usable as **full library sync between devices**: device A
  pushes `full`, device B pulls `full` and can continue culling/developing
  the same frames.

Partial vs. complete transfer in the owner's words maps exactly to these
scopes: `web` = smaller sizes only; `full` = originals with all details.

Invariants that do not change with any of this: originals are never
overwritten or deleted unless a delete is explicitly requested
(`allow_deletes`, named files); a pull never replaces a local original;
conflicts are reported, never guessed; untracked albums are untouched on
every side.

## Cross-library push

Pushing an album from the open library to a remote *belonging to another
library* is allowed: the app reads that library's remote config (it has the
path from the registry; the credentials stay in that library's catalog) and
performs a normal push against it. The receiving library does **not** have
to pull it — the album then simply exists on that remote, visible in the
receiving library's `remote_albums()` listing as a remote-only album, and
its owner pulls it if and when they want it locally. Provenance (which
library pushed) is recorded in the push, so the UI can label foreign
content. This is the "push can be remote-only" behavior: presence on a
remote does not imply local presence in any library.

## Master catalog (future — design constraint now)

A later service that aggregates, across all devices: which libraries exist,
their remotes, per-(album, remote, scope) sync status, and per-device
last-seen. Through it the owner can *request* that a device push part or
all of a library, and another device can pull it (partially, e.g. a tablet
pulling `web` scope for review, or fully for management).

What present-day schema work must therefore keep:

- **Stable ids**: every library gets a `library_id` (random, in settings)
  and every remote a stable `remote_id`; outcomes and manifests carry them
  so an aggregator can attribute state without guessing.
- **Per-(album, remote) state**, never global: subscriptions, baselines and
  publish records are keyed by remote/target — already required for
  multiple remotes, and exactly what a master catalog will read.
- **Requests are advisory**: the master catalog asks a device to push/pull;
  the device executes through the same `Session` methods with the same
  invariants. No new write path to a library ever bypasses the core.

## XMP: interchange, not source of truth

The owner asked whether edits should live in XMP and whether XMP is a
sufficient public format. Decision:

- **XMP is the interchange format, the catalog stays the source of truth.**
  XMP (ISO 16684-1) is genuinely public and every serious tool reads the
  standard fields: `xmp:Rating`, `xmp:Label`, `dc:subject` (keywords),
  `tiff:Orientation`. Those we read on import (Lightroom writes sidecars
  for RAW) and write on export, so triage work survives a move to or from
  any other tool.
- **The develop stack cannot be public in XMP.** There is no standard
  vocabulary for tone/geometry operations — even Lightroom's own
  `crs:` develop namespace is proprietary and other tools ignore it. Our
  stack goes into the sidecar under a `gpp:` namespace (versioned JSON,
  same bytes as `edits.stack_json`) — readable by us on any machine,
  harmlessly opaque to everyone else. A render key cannot be derived by a
  foreign tool anyway.
- **Why not make XMP primary storage**: per-photo sidecar files cannot hold
  album membership/order (cross-photo), sync baselines, or the tag
  normalization the catalog does; and a thousand tiny files are the
  slow/fragile path for every query the grid makes. SQLite stays primary;
  sidecars are (re)generated from it — they ride along in `full`-scope
  sync and make a library reconstructible from its files alone
  (catalog lost → re-import reads the sidecars back).

## Schema phasing

- **V4** (phase 1): `lr_links` / `lr_album_links` provenance for Lightroom
  import; `photo_tags` finally wired (was schema-only).
- **V5** (phase 2): `remotes` (id, name, target, token), `publish_targets`,
  `album_sync` → PK (album_path, remote_id) + `scope`,
  `sync_state` → PK (remote_id, kind, key), `published_files` →
  (target_id, album_path, filename), `library_id` in settings. Migration
  folds today's single remote/dest into row #1 of each; behavior after
  migration is identical until a second remote is added.
