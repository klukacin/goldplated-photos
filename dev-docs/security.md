# Security — the threat model and the invariants

Who the attackers actually are, for a photographer's gallery and desktop app:

1. **Anyone with a URL** — guessing paths to protected albums and their media.
2. **A malicious or malformed photo file** — off a client's card, from a second
   shooter, from a download. EXIF strings, dimensions, truncated data.
3. **Any web page the photographer visits** — which can reach loopback, where
   the admin panel listens.
4. **A hostile or compromised sync server / MITM** — the remote manifest is a
   document the server writes.
5. **A caller of the C ABI that gets it wrong** — null pointers, bad UTF-8,
   re-entrancy, panics that must never unwind into C.

Each has been probed by adversarial review with tests watched failing first.
What follows are the invariants that came out of that — change them only with
a test proving the replacement holds.

## Album access (gallery)

- Three access types: public, `password`, `shareToken` (secret link). A share
  token is **only ever honoured where it is explicitly set** — presenting an
  ancestor's token does not open a separately locked child. **A grant stops at
  the nearest lock**, whether the grant came from a cookie or a share link.
- The `album-access` cookie is `base64url(json).hmac`, HMAC-SHA256 with
  `ACCESS_SECRET` (min 16 chars; generated into a 0600 `.access-secret` file
  when unset, so sessions survive restarts). Forged/unsigned cookies parse to
  nothing. `httpOnly`, `sameSite: strict`, `secure` in prod, 24 h.
- Unlock is timing-safe compare + per-IP rate limiting (X-Forwarded-For aware),
  and the form's `returnUrl` is reduced to a same-site path (`safeReturnUrl`) —
  no `//host`, no `/\host`, no control characters — because an open redirect
  off the photographer's own domain is what makes a phishing link convincing.
- **Every media route re-checks access** (`resolveFileAccess`): originals,
  thumbnails, EXIF, video-info, watermark, ZIP download. Never add a media
  route without it. Hidden/locked inherit through the album chain everywhere
  content is listed (tag pages, search).

## Untrusted bytes

- **EXIF strings ride inside image files.** Everything interpolating them into
  markup goes through escaping (`src/lib/media-info.ts`, unit-tested) or DOM
  text nodes. A photo whose `Model` is `<img onerror=…>` must not run script
  on the origin where the access cookie lives.
- Hostile images: the `image` crate's allocation limits refuse decompression
  bombs; empty/truncated/garbage files return errors, are catalogued as
  `undecodable`, and never reach the site. Frontmatter written by the core
  escapes control characters and round-trips (`yaml_scalar`/`unquote`) — album
  titles and filenames come from pulls and cards, and one raw control byte
  used to fail the entire site build.
- Proofing submissions: body size capped before parse, filenames validated
  against the album's real photo list, rate-limited after auth so it cannot be
  starved, stored under slugified names. Spreadsheet formula injection in the
  CSV export is neutralised.

## The admin panel

Loopback is not a boundary; the fetch-metadata guard is (see
[gallery.md](gallery.md)). `resolveSafe` + `safeFilename` stop traversal and
dotfile writes; the script runner is a whitelist; upload filters are per-type
with size caps.

## Sync

Fail closed: no `SYNC_TOKEN` ⇒ 503. Constant-time compare. Server-supplied
paths validated on every pull (`accepts_remote_path`) *and* at the transfer
layer (`local_under`) — the guarantee belongs to the transfer, not to whichever
caller happens to precede it. `.meta/` excluded both directions. A pull never
overwrites a library original ([sync.md](sync.md)). The token goes out only as
an `Authorization` header, is never logged, and is not exposed through the FFI
(`has_remote_token` returns a bool instead).

## The C ABI

Every entry point catches unwinding; replies are always a JSON envelope, never
a crash. NULL session/method/args, non-UTF-8, unknown methods, unknown fields
(`deny_unknown_fields`), hostile numbers — all return typed error kinds, and
the session stays usable afterwards. `tests/foreign_caller.rs` drives the whole
matrix through the real ABI.

## Secrets inventory

| Secret | Where | Rules |
|---|---|---|
| `ACCESS_SECRET` | `.env` / `.access-secret` (0600, gitignored) | min 16 chars; without it sessions are file-persisted random |
| `SYNC_TOKEN` | `.env` server-side; catalog `settings` client-side | min 16 chars; endpoints closed without it |
| Album `password` / `shareToken` | frontmatter + catalog | plaintext by design (simple protection); share tokens CSPRNG-generated |
| Deploy credentials | `.env` | never in the repo |

Passwords in frontmatter are deliberate, documented, simple protection — not
cryptographic. Do not "upgrade" them without deciding what the gallery's
threat model actually needs.
