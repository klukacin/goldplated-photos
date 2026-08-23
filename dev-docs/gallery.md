# The gallery and the admin panel

The Astro site is the delivery surface; the admin panel is a local-only CMS
that edits the same content tree. Neither knows the desktop app exists — they
meet only at `src/content/albums/**`.

## Gallery (Astro, SSR)

`output: 'server'` with the Node standalone adapter. Public listing pages
prerender; **anything access-controlled is SSR** so a protected album's image
URLs never appear in page source. The route map, component list and
keyboard-shortcut reference live in `CLAUDE.md` (they double as the operational
quick-reference); this document covers the machinery.

### The media pipeline

Originals are served by `src/pages/albums/[...path].ts`; everything a browser
*displays* goes through `/api/thumbnail`:

- Three sizes (400/1200/1920 px), cached under the album's own
  `.meta/thumbnails/`, invalidated by mtime, generated behind a semaphore so a
  cold album does not fork-bomb Sharp.
- Content negotiation serves WebP where accepted, JPEG otherwise — never the
  source format. That matters because **HEIC is accepted as a source but no
  Chrome or Firefox can display it**; the displayability rule lives in
  `src/lib/image-formats.ts` and every surface (lightbox original-quality
  toggle, og:image, admin previews, error fallbacks) is required to route
  through it. The admin's copies of the format lists are held to the TS module
  by a drift test.
- Public assets (`public/home/hero`, cards, landing) are served raw with no
  conversion anywhere, so their upload endpoints refuse formats a browser
  cannot paint.

Per-album metadata (dimensions, EXIF summary, blur LQIPs) is cached in
`<album>/.meta/index.json`, keyed by file size+mtime, pruned when files
disappear. Search reads this cache; it never walks pixels at request time.

### Access control

One implementation, used by the album page **and every media route**:
`resolveAlbumAccess` / `resolveFileAccess` in `src/lib/access.ts`, pure logic
in `access-core.ts` (unit-tested). Never add a media route without calling
them. The model, the cookie format and the inheritance rules are in
[security.md](security.md).

### Sync endpoints

`/api/sync/manifest` and `/api/sync/file` exist for the desktop app's
`HttpTransport` and follow the same fail-closed pattern as everything else:
no `SYNC_TOKEN` (min 16 chars) ⇒ 503, constant-time compare, paths validated
before disk, uploads hash-verified through a temp file, a server-side hash
cache keyed by `(size, mtime)` — see [sync.md](sync.md) for why.

## Admin panel (`admin/`, Express on :4444)

Local only, never deployed. Loopback binding is **not** the security boundary —
every page the photographer visits can reach loopback too, and `cors()` does
not stop requests (it only gates response *reads*, after the handler ran). The
boundary is a fetch-metadata guard before every route: `Sec-Fetch-Site`
must be `same-origin` or `none`. That header is browser-forbidden (no page can
forge it), which is also why a `same-origin` request skips the Origin
allow-list — the panel stays reachable at LAN names the list cannot know.
Non-browser clients send neither header and still work; the panel stays
scriptable. Without this guard, `<img src="http://localhost:4444/api/tools/run/deploy">`
on any web page ran a production deploy.

Other hardening that is load-bearing: `resolveSafe` (traversal), `safeFilename`
(basename only, no dotfiles), multer size/extension filters per upload type,
collision-suffixing instead of silent overwrite, a whitelisted script runner
(nothing user-supplied reaches `spawn`), and `ADMIN_PORT` so tests take a free
port instead of fighting a running panel.

The admin frontend is vanilla JS + a vendored CodeMirror (works offline). It
builds untrusted text (EXIF strings, proofing comments) exclusively with
`createElement`/`textContent`; the CSV export neutralises spreadsheet formula
injection with a leading apostrophe.

## Things that look like bugs but are decisions

- **`.meta/` is server-owned.** Deploys and syncs must exclude it in both
  directions; proofing submissions live there and are pull-only.
- **Locked albums are listed** on tag pages (title + lock, no cover) — that is
  deliberate; *hidden* ones are not. Lock and hidden both inherit from
  ancestors (`resolveChainVisibility`).
- **`Flag::Reject` removes a photo from the next publish** by default
  (`exclude_rejected: true`) — a reject is not a neutral note.
- The gallery's search only ever matches fully public chains; protected
  content is invisible to it by construction, not by filtering at render.
