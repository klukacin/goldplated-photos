# gpp-ffi

A C ABI door onto `gpp-core`, so a native client that is not written in Rust can
drive the same catalog, develop pipeline, publisher and sync engine the desktop
app does.

`gpp-core` was already portable — no GUI, no async runtime, `Send + Sync`, and
it compiles for `aarch64-apple-ios`. What it had no way to offer was a handle: a
Swift iPad app or a Kotlin Android app had nothing to hold. This crate is that
handle and nothing else. It contains no behaviour of its own; every call lands
on a `Session` method.

---

## The whole API

```c
GppSession *gpp_session_new(void);
char       *gpp_call(GppSession *session, const char *method, const char *args_json);
void        gpp_string_free(char *ptr);
void        gpp_session_free(GppSession *ptr);
```

`Session` has around forty-five methods and all of them already serialise —
that is what the Tauri shell has been relying on since it was written. So rather
than hand-writing forty-five C signatures that would drift the first time a
field was added, everything goes through one JSON call. The header is four
declarations that have no reason to change, and adding a method to the core is a
match arm here rather than an ABI break for everyone downstream.

The price is real and worth naming: type checking happens at run time inside
serde, not at compile time in the caller's language. Two things soften it. Every
argument struct sets `deny_unknown_fields`, so a misspelled key is a returned
error rather than a value that silently did nothing. And the reply is always one
of exactly two shapes:

```json
{"ok": <the method's return value, or null for a method that returns nothing>}
{"error": "album not found: 2026/x", "kind": "album-not-found"}
```

Branch on `kind`; show `error`. The tags are stable and written out by hand in
`dispatch.rs`, so renaming a Rust enum variant cannot quietly change what a
shipped app switches on:

| from the core | from this boundary |
|---|---|
| `database`, `io`, `image`, `serde`, `invalid-path`, `album-not-found`, `photo-not-found`, `album-exists`, `unsupported`, `source-offline`, `source-mismatch`, `sync-conflict`, `other` | `null-pointer`, `invalid-utf8`, `bad-arguments`, `unknown-method`, `panic` |

`source-offline` is the one worth branching on rather than only showing: it
means a registered drive is not attached, nothing is wrong with the library, and
the remedy — plug it in — is something the person reading can act on. Its
message names the source.

---

## Building

```sh
cd desktop
cargo build -p gpp-ffi --release
```

You get all three shapes, because the three consumers want different ones:

| Artifact | For |
|---|---|
| `libgpp_ffi.dylib` / `.so` / `.dll` | anything that `dlopen`s — Python `ctypes`, Node FFI, a desktop Kotlin app |
| `libgpp_ffi.a` | iOS, where the App Store will not take a loose dynamic library |
| `libgpp_ffi.rlib` | Rust callers, and this crate's own tests |

For a device build, add the target and pass it through:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build -p gpp-ffi --release --target aarch64-apple-ios
```

The header is `include/gpp_ffi.h`, checked in and hand-written. The usual
generator, cbindgen, is MPL-2.0 (it is a Mozilla project, like UniFFI), and this
project keeps every Rust dependency — build-time ones included — under
MIT / Apache-2.0 / BSD / CC0. Four declarations are cheaper to maintain by hand
than a licence exception is to argue for, and `tests/header_matches_the_code.rs`
fails the build if the header and the source stop agreeing.

---

## Using it from Swift

Point a bridging header at `include/gpp_ffi.h` (or, for a Swift package, ship it
as a `systemLibrary` target with a modulemap), link the static library, and the
four functions appear as Swift functions.

A thin wrapper is worth writing once. This is the whole of it:

```swift
import Foundation

/// Raised when the core returns `{"error": ..., "kind": ...}`.
struct GppError: Error, LocalizedError {
    let kind: String
    let message: String
    var errorDescription: String? { message }
}

final class GppLibrary {
    private let session: OpaquePointer

    init() throws {
        guard let s = gpp_session_new() else {
            throw GppError(kind: "other", message: "could not create a session")
        }
        session = OpaquePointer(s)
    }

    deinit {
        // Exactly once, and nothing may still be inside `call`.
        gpp_session_free(UnsafeMutablePointer(session))
    }

    /// One trip across the boundary. `args` is encoded to JSON, the reply is
    /// decoded, and the buffer the core allocated is handed straight back.
    @discardableResult
    func call(_ method: String, _ args: [String: Any] = [:]) throws -> Any? {
        let argsData = try JSONSerialization.data(withJSONObject: args)
        let argsJSON = String(decoding: argsData, as: UTF8.self)

        guard let reply = gpp_call(UnsafeMutablePointer(session), method, argsJSON) else {
            // Documented never to happen; treated as fatal rather than ignored.
            throw GppError(kind: "other", message: "gpp_call returned NULL")
        }
        defer { gpp_string_free(reply) }   // the one rule: free what you are given

        let text = String(cString: reply)
        let object = try JSONSerialization.jsonObject(with: Data(text.utf8))
        guard let envelope = object as? [String: Any] else {
            throw GppError(kind: "other", message: "unexpected reply: \(text)")
        }
        if let message = envelope["error"] as? String {
            throw GppError(kind: envelope["kind"] as? String ?? "other", message: message)
        }
        return envelope["ok"]
    }
}
```

And then the app is ordinary Swift:

```swift
let gpp = try GppLibrary()

try gpp.call("open_library", ["path": libraryURL.path])

let picks = try gpp.call("photos", ["filter": ["minRating": 4, "flag": "pick"]])
    as? [[String: Any]] ?? []

try gpp.call("set_rating", ["ids": picks.map { $0["id"]! }, "rating": 5])

try gpp.call("create_album", ["album": ["path": "2026/weddings/ana-ivan",
                                        "title": "Ana & Ivan"]])
try gpp.call("add_to_album", ["path": "2026/weddings/ana-ivan",
                              "photo_ids": picks.map { $0["id"]! }])

let link = try gpp.call("generate_share_link", ["path": "2026/weddings/ana-ivan"])
```

`gpp_call` is blocking and some calls are long — `import`, `publish`,
`push_album`, `sync_all_tracked` can each run for minutes. The session is
`Send + Sync`, so run those on a background queue and keep reading from the main
one; the only rule is that no call may be in flight when the session is freed.
An import started that way can be abandoned from the main queue with
`cancel_import`, which takes no lock and so does not wait for the very call it
is trying to end.

Kotlin over JNA is the same shape: `Pointer gpp_session_new()`,
`Pointer gpp_call(...)`, then `getString(0)` and `gpp_string_free`. Do not let
JNA's `String` return mapping tempt you — it copies and then leaks the original,
because it has no idea the core wants it back.

---

## Ownership, in one place

| Function | Who owns what |
|---|---|
| `gpp_session_new` | You own the result. Free with **exactly one** `gpp_session_free`. Returns `NULL` on failure — the only case with nothing to free. |
| `gpp_call` | You own the returned string. Free with **exactly one** `gpp_string_free`. **Never returns `NULL`.** Do not use the host's `free()`: the library may not share your allocator. |
| `gpp_string_free` | Takes ownership. `NULL` is a no-op. Twice is undefined behaviour — heap corruption, usually noticed somewhere else entirely. |
| `gpp_session_free` | Takes ownership. `NULL` is a no-op. Twice is undefined behaviour, as is any `gpp_call` that starts after — or is still running during — this one. |

Nothing you pass in is retained. `method` and `args_json` are borrowed for the
duration of the call and you may free them the moment it returns. Freeing a
session does not free strings it produced; those are separate allocations and
each still needs its own `gpp_string_free`.

**Panics never cross the boundary.** Unwinding out of an `extern "C"` function
is undefined behaviour, so every entry point catches. A panic in the core comes
back as `{"error": ..., "kind": "panic"}`. Treat that as fatal to the session:
the lock inside it is now poisoned, so it is still safe to touch but every later
call will fail. Free it and open the library again.

---

## Naming

Method names are `Session`'s method names and argument keys are its parameter
names, both unchanged — there is no second vocabulary to keep in step, and
anyone reading `session.rs` already knows what to type.

That makes almost everything `snake_case`, arguments and replies alike. Three
nested types are `camelCase` instead — `PhotoFilter`, `NewAlbum` and
`AlbumUpdate` — because the web UI made them that way long before this crate
existed. It is a genuine seam and it belongs to the core, not to this door;
renaming it here would only move the surprise onto the Tauri shell.

---

## Methods

Argument objects are shown by their keys. Anything marked *(optional)* may be
omitted; a call with no arguments accepts `NULL`, `""` or `"{}"`.

### Library

| Method | Arguments | Returns |
|---|---|---|
| `open_library` | `path` | `LibraryStatus` |
| `is_open` | — | `bool` |
| `status` | — | `LibraryStatus` |
| `image_dirs` | — | `[string]` — the directories a viewer must be allowed to read |

### Import

| Method | Arguments | Returns |
|---|---|---|
| `import` | `dir` *(optional; omit to walk every registered source)* | `ImportSummary` |
| `cancel_import` | — | `null` |
| `prune` | — | `int` — catalog rows dropped because the file is gone |

A `dir` inside a registered source (see below) is catalogued **where it lies**;
one outside every source is copied into the library first, because a catalog row
has no way to name a file no source reaches. Omit `dir` and the run walks the
primary and every referenced source in turn, skipping any whose drive is not
attached — those are named in `ImportSummary.notes`, never raised as errors.

`prune` only examines sources that are online. A photograph on an unplugged
drive is exactly as present as it was yesterday, and dropping its row would take
its rating, flags, album memberships and adjustments with it.

`import` blocks for as long as the card takes. `cancel_import` is the one call
worth making while another is still running: it raises a flag the import reads
between files and never touches the library, so it answers immediately from a
second thread. The run then returns its partial `ImportSummary` with
`cancelled: true` — every other count in it is a partial tally, and a caller
that ignores the flag will announce a finished import that never finished. What
had already landed stays landed, and importing the same folder again finishes
the job. Calling it when nothing is running is harmless: the next `import`
clears the flag before it reads a file.

### Sources (schema v6)

A **source** is a root a catalogued photograph may live under. The library root
is one — the *primary* — and any folder registered here is another; files there
are **referenced**: read, hashed, thumbnailed, developed and published, never
copied, never moved, never written to. That is what makes a Lightroom library
importable without migrating a terabyte.

| Method | Arguments | Returns |
|---|---|---|
| `sources` | — | `[SourceInfo]` (`id`, `name`, `path`, `kind`, `volume_hint`, `is_primary`, `online`, `photo_count`) |
| `add_source` | `path`, `name` *(optional; defaults to the folder's own name)*, `kind` *(optional: `"internal"` \| `"external"` (default) \| `"network"`)* | `id` |
| `remove_source` | `id`, `drop_photos` *(optional, default `false`)* | `int` — catalog rows dropped. **Never deletes a file.** |
| `relocate_source` | `id`, `new_path` | `null` |

`online` is probed at the moment of the call, never cached — that is the whole
question a drive that comes and goes asks. Offline, listing and thumbnails keep
working (thumbnails are content-addressed and live on the primary), while
anything that must open an original fails with the `source-offline` tag and a
message naming the source; batch operations report those per item instead
(`PublishResult.offline`, `XmpExportOutcome.offline`, `PushOutcome.failed`).

`add_source` refuses a folder that overlaps one already registered, in either
direction — including the library root. Two roots over one file would give it
two identities, and then a prune, a publish and a sync each disagree about what
the library holds.

`remove_source` refuses a source that still holds catalog rows unless
`drop_photos` says so in as many words: those rows carry ratings, flags, album
memberships and develop stacks that exist nowhere else. The primary cannot be
removed or relocated — it is where the catalog and every thumbnail live.

`relocate_source` is for a drive that mounted somewhere else. It re-hashes a
handful of that source's own catalogued files at the new path and refuses, with
the `source-mismatch` tag and the offending filename, if they are not there or
differ. Pointing a source at last year's backup would otherwise re-attach every
row to the wrong negatives, quietly.

### Lightroom

| Method | Arguments | Returns |
|---|---|---|
| `lr_scan` | `lrcat_path` | `LrScanReport` — root folders (with per-folder `in_place` / `can_reference` verdicts and missing-file counts), collections, keyword count, catalog id. Writes nothing. |
| `lr_import` | `lrcat_path`, `options` *(optional `LrImportOptions`)* | `LrImportReport` |

`LrImportOptions` is snake_case like the rest of this door: `dest_subdir`
(where copied files land, default `"lr"`), `collections` (paths or `*` globs
narrowing which collections map to albums; photos import regardless),
`album_prefix`, `collision` (`"auto"` \| `"merge"` \| `"suffix"` — what to do
when a collection's album path is already taken by an unlinked album),
`dry_run` (compute the full report, write nothing), and the placement pair
below.

**Placement.** `mode` says where each root folder's files end up:

| `mode` | Meaning |
|---|---|
| `"auto"` (default) | In place where the root already lies inside a source; copy in otherwise. What every import did before referencing existed. |
| `"reference"` | Register the root folder as a source (named after it) and catalogue its files where they lie. **Nothing is copied.** |
| `"in-place"` | Catalogue where the files are; on a root outside every source this registers one and says so in `conflicts`. |
| `"copy"` | Always copy into the library under `dest_subdir`. |

`roots` overrides `mode` per root folder: `[{"root_id": 2, "mode": "copy"}]`,
where `root_id` is the `AgLibraryRootFolder` id `lr_scan` reports. A
photographer keeps this year on the laptop and eight years on a NAS; those are
two answers, not one.

`LrImportReport` gains `photos_referenced` (the subset of `photos_in_place`
that landed on a source other than the library root — the number that answers
"how much did I import without copying a byte") and `sources_registered`. A
root that cannot become a source (it contains the library, or another source)
falls back to copying, with the reason in `conflicts`.

The `.lrcat` is copied under the library's `.gpp` directory and only the copy
is read — Lightroom can stay open, and the original is never touched. Both
calls block like `import` does; run `lr_import` on a background thread, and
`cancel_import` stops it the same way. Re-running with the same catalog syncs
instead of duplicating: photos and collections are remembered per source
catalog, renames in Lightroom follow (unless the album was renamed locally too
— reported in `conflicts`, nothing moved), and Lightroom-side deletions are
reported in `lr_deleted`, never propagated.

### Photos

| Method | Arguments | Returns |
|---|---|---|
| `photos` | `filter` *(optional `PhotoFilter`, camelCase)* | `[Photo]` |
| `photo` | `id` | `Photo` |
| `photo_path` | `id` | `string` — absolute path of the original |
| `thumbnail_path` | `id`, `size` (`"small"` \| `"medium"` \| `"large"`) | `string` |
| `set_rating` | `ids`, `rating` (0–5) | `int` — photos changed |
| `set_flag` | `ids`, `flag` (`"none"` \| `"pick"` \| `"reject"`) | `int` |
| `set_color_label` | `id`, `label` *(optional; `null` clears)* | `null` |

### Develop

Adjustments are non-destructive: the original file is never opened for writing.
An op is `{"op": "<kind>", ...}` — `exposure` (`ev`, in stops), `contrast`,
`saturation`, `temperature`, `tint`, `highlights`, `shadows` (each `amount`,
-100..100), `black-and-white`, `rotate` (`quarter_turns`), `flip-horizontal`,
`flip-vertical`, `crop` (`x`, `y`, `w`, `h`, fractions of the frame).

| Method | Arguments | Returns |
|---|---|---|
| `photo_edits` | `id` | `EditStack` — never absent; an untouched photo has an empty `ops` |
| `set_photo_edit` | `ids`, `op` | `int` — photos changed |
| `rotate_photos` | `ids`, `quarter_turns` (signed; positive is clockwise) | `int` |
| `toggle_photo_edit` | `ids`, `op` | `int` |
| `clear_photo_edit` | `ids`, `kind` (e.g. `"exposure"`) | `int` |
| `reset_photo_edits` | `ids` | `int` |

Two of those exist because a button is not a slider. `rotate_photos` is
*relative* — a selection can hold photos at different angles, and "rotate right"
has to mean the same thing to each of them, which setting a `rotate` op would
not. `toggle_photo_edit` is for the flips, the only adjustments with no zero to
set: `set_photo_edit` drops the existing mirror and pushes an identical one
straight back, so it could never undo one.

Both write through the canonical framing — one horizontal mirror followed by
quarter turns, the eight ways a rectangle can be set down — so what comes back
from `photo_edits` is not always the op you sent. A `flip-vertical` toggle is
stored as `flip-horizontal` plus a half turn, and `flip-vertical` is never
written. Read orientation by folding the whole op list, never by looking for a
particular op.

### Albums

| Method | Arguments | Returns |
|---|---|---|
| `albums` | — | `[AlbumSummary]` |
| `create_album` | `album` (`NewAlbum`, camelCase) | `Album` |
| `update_album` | `path`, `update` (`AlbumUpdate`, camelCase; `null` clears a field, omitting it leaves it) | `Album` |
| `move_album` | `from`, `to` | `Album` |
| `delete_album` | `path` | `null` |
| `album_photos` | `path` | `[Photo]` |
| `add_to_album` | `path`, `photo_ids` | `int` |
| `remove_from_album` | `path`, `photo_ids` | `int` |
| `reorder_album` | `path`, `photo_ids` | `null` |
| `generate_share_link` | `path` | `string` — a fresh share-token secret |

### Publish

| Method | Arguments | Returns |
|---|---|---|
| `publish_target` | — | `PublishTarget` (`dest`, `min_rating`) |
| `set_publish_target` | `target` | `null` |
| `publish` | `album_path` *(optional; omit for every album)* | `[PublishResult]` |
| `sync_plan` | — | `SyncPlan` |

### Remote and sync

| Method | Arguments | Returns |
|---|---|---|
| `remote_dir` | — | `string \| null` |
| `set_remote_dir` | `dir` — a path, or an `http(s)` URL for the sync API | `null` |
| `has_remote_token` | — | `bool` |
| `set_remote_token` | `token` | `null` |
| `remote_albums` | — | `[RemoteAlbum]` — everything syncable, from either side |
| `album_subscriptions` | — | `[AlbumSubscription]` |
| `track_album` | `album_path`, `direction` (`"push"` \| `"pull"` \| `"both"`) | `null` |
| `untrack_album` | `album_path` | `null` |
| `plan_album_sync` | `album_path`, `direction` | `SyncPlan` — a preview; moves nothing |
| `pull_album` | `album_path` | `PullOutcome` |
| `push_album` | `album_path`, `allow_deletes` *(optional, default `false`)* | `PushOutcome` |
| `sync_album` | `album_path`, `direction`, `allow_deletes` *(optional)* | `SyncOutcome` |
| `sync_all_tracked` | `allow_deletes` *(optional)* | `[[album_path, SyncOutcome]]` |

Every call in this table also accepts an **optional `remote_id`** (schema v5:
a library can know several remotes). Omitted, the default remote is used —
which is exactly the single remote a pre-v5 catalog migrated into, so every
old call keeps working unchanged. `track_album` additionally accepts an
optional `scope`: `"web"` (default — the published tree, today's behaviour) or
`"full"` (originals and per-album metadata as well, under the reserved
`__gpp_full__/` namespace). `publish` accepts an optional `target_id` the same
way.

Deletions on the server are withheld unless `allow_deletes` is passed. What was
withheld comes back in `withheld_deletes` so you can name the files, ask, and
run again — see `dev-docs/sync.md`.

### Remotes and publish targets (schema v5)

| Method | Arguments | Returns |
|---|---|---|
| `remotes` | — | `[RemoteInfo]` (`id`, `name`, `target`, `token`, `is_default`, …) |
| `add_remote` | `name`, `target`, `token` *(optional)* | `id` — the first added becomes the default |
| `update_remote` | `id`, `update` (`name?`, `target?`, `token?` — `null` clears) | `null` |
| `remove_remote` | `id` | `null` — drops its subscriptions and baselines **locally**; the server is untouched |
| `set_default_remote` | `id` | `null` |
| `publish_targets` | — | `[PublishTargetInfo]` |
| `add_publish_target` | `name`, `dest_root`, `min_rating` *(optional)* | `id` |
| `update_publish_target` | `id`, `update` (`name?`, `dest_root?`, `min_rating?` — `null` clears) | `null` |
| `remove_publish_target` | `id` | `null` |
| `set_default_publish_target` | `id` | `null` |
| `read_library_remotes` | `library_root` | `[RemoteInfo]` of **another** library, read-only — its catalog is never migrated |
| `push_album_to` | `album_path`, `target`, `token` *(optional)*, `scope` *(optional)* | `ForeignPushOutcome` — a stateless push: no baselines, never deletes, names every overwrite, carries this library's `library_id`/`library_name` for provenance |

### Interchange

| Method | Arguments | Returns |
|---|---|---|
| `export_xmp` | `album_path` *(optional; omit for the whole catalog)* | `XmpExportOutcome` (`written`, `skipped_foreign`, `missing`, `offline`) |

Sidecars carry `xmp:Rating`, `xmp:Label`, `dc:subject`, `tiff:Orientation` —
the fields every serious tool reads — plus the develop stack verbatim under
the versioned `gpp:` namespace. A sidecar without that namespace belongs to
another tool and is never overwritten. Each one is written **beside its own
original**, so a referenced photograph's sidecar lands on its own drive rather
than being gathered into the library; a source that is not attached is named in
`offline` and nothing is written for it.

The Rust-side list is `gpp_ffi::METHODS`, and a test asserts that every name in
it dispatches.

---

## What is deliberately not here

**Progress callbacks.** `Session::import` takes an `on_progress` closure and the
Tauri shell streams it to the UI. There is no honest way to hand a Rust closure
to C, and bolting a function-pointer-plus-userdata channel onto this crate would
be a second, unrelated door. Through this one an import is synchronous and
silent: run it off the main thread and show an indeterminate spinner. Silent is
not the same as unstoppable, though — `cancel_import` crosses in the other
direction, so the spinner can still have a Cancel next to it. If a real
client needs a progress bar badly enough, the right shape is a separate
`gpp_call_with_progress` taking `void (*)(const char *json, void *user)` — not a
callback smuggled through the JSON.

**`Session::remote_token`.** Settable, never readable through
`has_remote_token`, which reports only whether one is on file — the line the
Tauri shell draws for the single-remote panel. Note that `remotes` and
`read_library_remotes` *do* include each remote's `token`: a cross-library
push exists precisely to carry a credential out of a catalog and into a
transport the caller builds, and the catalog stores it in plain text anyway.
Treat those results accordingly.

**Everything else on `Session` is reachable.** The table above is the whole of
it.

---

## Tests

```sh
cd desktop && cargo test -p gpp-ffi
```

`tests/foreign_caller.rs` drives the library the way C does — NUL-terminated
byte strings in, `char *` out, `gpp_string_free` on every reply, every session
released — including the failure cases that matter most at a boundary: a `NULL`
session, a `NULL` method, input that is not UTF-8, malformed JSON, a misspelled
argument key, an unknown method name, and a call that makes the core itself
fail. `tests/header_matches_the_code.rs` guards the hand-written header against
drift. The panic guard is unit-tested in `src/lib.rs`, because the only way to
prove a panic is caught is to cause one and no method in the table exists to
fail on demand.
