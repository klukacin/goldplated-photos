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
| `database`, `io`, `image`, `serde`, `invalid-path`, `album-not-found`, `photo-not-found`, `album-exists`, `unsupported`, `sync-conflict`, `other` | `null-pointer`, `invalid-utf8`, `bad-arguments`, `unknown-method`, `panic` |

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
| `import` | `dir` *(optional; defaults to the library root)* | `ImportSummary` |
| `cancel_import` | — | `null` |
| `prune` | — | `int` — catalog rows dropped because the file is gone |

`import` blocks for as long as the card takes. `cancel_import` is the one call
worth making while another is still running: it raises a flag the import reads
between files and never touches the library, so it answers immediately from a
second thread. The run then returns its partial `ImportSummary` with
`cancelled: true` — every other count in it is a partial tally, and a caller
that ignores the flag will announce a finished import that never finished. What
had already landed stays landed, and importing the same folder again finishes
the job. Calling it when nothing is running is harmless: the next `import`
clears the flag before it reads a file.

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

Deletions on the server are withheld unless `allow_deletes` is passed. What was
withheld comes back in `withheld_deletes` so you can name the files, ask, and
run again — see `ARCHITECTURE.md` §6.3.

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

**`Session::remote_token`.** Settable, never readable: `has_remote_token`
reports only whether one is on file. This is the line the Tauri shell already
draws, and there is no reason for a secret that has been written into the
catalog to travel back out to a caller that already had it.

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
