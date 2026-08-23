# Extending the system — recipes

Each recipe lists every place a change must land. The lists are the point:
most historical defects here were changes that reached *some* of the places.

## Add an image format (core)

1. `media::IMAGE_EXTENSIONS` — adding an extension **promises `media::decode`
   can open it**. If the `image` crate can't, add a decode arm (the HEIF arm
   is the template) — pure Rust only, licence-clean (MIT/Apache/BSD/CC0; no
   LGPL, no MPL, even build-time).
2. `media::read_dimensions` — a cheap header path if the format has one; a
   full decode if not (say so in the doc comment, it changes import speed).
3. **Decide displayability.** Can Chrome/Firefox paint it? If not, wire it
   into `publish::published_filename` (publish as JPEG, name stable across
   develops) and into the gallery's `src/lib/image-formats.ts`.
4. A real fixture in `desktop/crates/gpp-core/tests/fixtures/` — in the codec
   the real world produces, not the nearest one your tools can write (AV1-in-
   HEIF proves nothing about iPhone HEVC). **Negate it in `.gitignore`** or CI
   fails on a file that exists on your disk.
5. Tests: classify, decode, dimensions, publish naming. Gallery side: the
   extension lists in `image-formats.ts` + the admin copies (the drift test
   will hold you to it), upload filters, and a thumbnail-endpoint check.

## Add an edit operation (develop)

1. `EditOp` variant + `kind()` + `is_identity()` (identity ⇒ the op is dropped,
   keeping untouched photos on their original render key) + `clamped()`.
2. `apply` — tone ops go in `apply_tone` (pure, per pixel); geometry ops are a
   different animal entirely: they must compose through `Framing`, never be
   edited in place. Read the canonical-orientation section of
   [core.md](core.md) *before* touching geometry; two shipped defects live in
   that history.
3. The render key changes automatically (it hashes the stack) — but if your op
   changes *encoding*, the encoder parameters belong in the key too.
4. UI: a slider def in `desktop/app/ui/app.js` goes through the develop queue
   (`developQueue.queue`, coalescing keyed by kind *and* photo ids); a
   relative control (like rotate) uses `queueRelative` so presses add up
   instead of the newest replacing the rest.
5. Tests: the maths in `develop.rs`; an end-to-end pass in
   `develop_end_to_end.rs`; geometry additions extend `geometry_order.rs`.

## Add a `Session` method

A capability is not shipped until all four doors open onto it:

1. `Session` method (`session.rs`) — owned serde types, ids not paths.
2. Tauri command (`app/src-tauri/src/lib.rs`) + registration in
   `generate_handler!` — check-shell fails on an unregistered `invoke()`.
3. FFI dispatch arm (`gpp-ffi/src/dispatch.rs`): snake_case argument struct,
   `deny_unknown_fields`, same envelope as the neighbours; hostile-argument
   coverage in `tests/foreign_caller.rs` (no reply may ever be
   `"kind":"panic"`). Update the crate README's method table.
4. CLI subcommand if it's photographer-facing (`gpp-cli`).

## Add a gallery API route

Non-negotiable: if it serves anything derived from album content, it calls
`resolveFileAccess`/`resolveAlbumAccess` before touching disk. Then: path
validation before filesystem access, `Cache-Control: private` for protected
content, a Vitest file that spawns the route and proves the deny path (401/404)
*and* the allow path. Grep `src/pages/api/thumbnail.ts` for the house pattern.

## Add a sync transport

Implement `sync::RemoteTransport` (4 methods: manifest/put/get/delete).
Everything above the trait — planning, conflicts, subscriptions, path
validation — is transport-agnostic and already tested; your job is only moving
bytes. Requirements: no spawning processes (portability contract), bounded
memory on `get` (cap oversized responses), and never trusting the manifest you
return — the engine validates it, but don't *depend* on that in your own code.
Run the `two_machines.rs` suite against your transport the way `FsTransport`
does.

## Add an admin capability

Route in `admin/server.js` (behind the fetch-metadata guard — it applies to
every route automatically; `resolveSafe` any path; multer filters for uploads),
frontend module under `admin/js/`, and an HTTP-level test that spawns the real
server (`ADMIN_PORT`) — the cross-site suite is the template.

## Add a feature flag

See [features.md](features.md) — flags have one source of truth per side, and
a flag that hides UI without disabling the route is not a flag, it's a
decoration.
