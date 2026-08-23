# Sync — moving albums between machines without losing anything

The failure mode this engine exists to prevent: machine B holds three albums
out of two hundred, runs a publish, and the server loses the other 197. That
is what `rsync --delete` does, and why the deploy script is for the *site*
while album content moves through this engine.

## Three-way state

Per file: `local_hash`, `synced_hash` (the last agreement), `remote_hash`.

| local | synced | remote | Meaning | Action |
|---|---|---|---|---|
| A | A | A | in sync | nothing |
| B | A | A | changed locally | push |
| A | A | B | changed remotely | pull |
| B | A | C | changed on both | **conflict → report, never guess** |
| — | A | A | deleted locally, on purpose | delete remote (needs `allow_deletes`) |
| — | — | A | never had it here | **leave alone** |
| A | — | — | new locally | push |

The last two rows are the distinction `rsync --delete` cannot make.

**`ForgetState` must be applied, not skipped.** Several triples mean "nothing
to transfer, but the books need settling" — both sides deleted, both sides
converged independently, both sides hold it with no baseline. Dropping those
rows (the original behaviour) had two proven consequences: a file another
machine republished got *deleted* (the stale baseline read as "I removed this
on purpose"), and some conflicts could never converge. `apply` settles the row
from the one observable fact — the local file's presence — and the planner
skips a `ForgetState` only when baseline and local already agree, so an
in-sync album still costs nothing.

## Per-album subscriptions decide scope

One row per album in `album_sync(album_path, direction)`: `push` (local wins,
server never overrules), `pull` (server wins, server never written — not even
deletions), `both` (the full table, conflicts surfaced). **An album with no
row is not synced at all** — not pushed, not pulled, not deleted, not listed.
That is what makes a laptop holding three albums safe against a server holding
two hundred.

Rules that were each a demonstrated defect first:

- **A subscription follows its album.** Rename/move carries it (and the
  sub-albums'); delete drops it. A stranded row used to abort the entire
  batch — one album renamed on Wednesday stopped Friday's shoot from syncing.
- **One album's failure never stops the batch**, and one file's failure never
  stops its album. Failures ride out through `SyncOutcome.failed` /
  `PushOutcome.failed`, per item, because a gallery arriving one frame short
  must not look like a clean push.
- **Paths travel whole.** Pushing `2026/weddings/ana-ivan` sends each
  ancestor's `index.md` first (one file at a time — a prefix *scope* on `2026`
  would sweep in other machines' albums); pulling walks parents first so a
  collection can never flatten. Scope matching is per whole segment
  (`2026extra` does not match `2026`).
- **Renaming a tracked album moves nothing on the remote.** The server holds
  the old path until someone deletes it with `allow_deletes`; deletions are
  never implicit.

## What a pull may and may not do

- **A pull only ever adds a photo to the library.** The library original is
  the negative — there is no second copy anywhere, and the plan's "local" side
  is the *published* tree (developed pixels), so the original was never part
  of the comparison. Where the server disagrees, the published copy takes the
  server's version and the original is left untouched, named in
  `PullOutcome.kept_originals` for a person to judge.
- **A remote manifest is a document the server writes**, so every path in it
  is validated (`sync::accepts_remote_path`) before either write a pull makes:
  no empty/dot/dot-dot segments, nothing absolute, no NUL, no backslash, no
  leading-dot names. Without that, a hostile server could plant an `.htaccess`
  in the tree `npm run deploy` rsyncs to the live host — invisible afterwards,
  because both manifests skip dot-names. Refused paths land in
  `PullOutcome.rejected`; the rest of the album still arrives.
- `.meta/` is server-owned (proofing submissions) and excluded from every
  manifest in both directions. Nothing can push into it; deploys must not
  delete it.
- Metadata (`index.md`, `body.md`) never conflicts on a pull: it is derived
  from the catalog the pull has just overwritten anyway, so holding it in a
  conflict would strand it there forever.

## Transports

One trait, two implementations — a folder (`FsTransport`: network share,
external drive) and the gallery's own HTTP endpoints (`HttpTransport`). The
HTTP side is `/api/sync/manifest` + `/api/sync/file`, guarded by `SYNC_TOKEN`
(min 16 chars; **unset they answer 503 rather than opening**), constant-time
token compare, paths validated before disk, uploads verified against
`X-Content-Blake3` and written through a temp file.

**Throughput is a solved problem, measured on a 1.1 GB / 60-photo shoot over
loopback** (so only our code is timed):

| | before | after |
|---|---|---|
| server manifest of the tree | 41.7 s | **0.01 s** |
| push with nothing to send | 76.5 s | **0.4 s** |
| first push of the whole 1.1 GB | 46.4 s | 43.9 s (25 MB/s) |
| push of 5 changed photos | — | **3.6 s** |

The gallery hashes with BLAKE3 *in JavaScript* at ~32 MB/s while the disk
reads at 2.3 GB/s; the fix was a server-side hash cache keyed by
`(size, mtime)`, filled as a side effect of upload verification. The cache can
only ever cost an unnecessary upload, never a skipped one. What remains is
upload verification at ~200 Mbit/s — above any consumer uplink, so not what a
real push waits on. The full measurement log, wrong turns included, is in
[sync-transport.md](sync-transport.md); read it before "optimising" the
transport, because the obvious ideas were measured and most of them were not
the bottleneck.
