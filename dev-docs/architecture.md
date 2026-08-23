# Architecture — the whole system

Goldplated Photos is three deliverables sharing one contract:

```
┌────────────────────┐   publish    ┌──────────────────────────┐
│  Desktop app        │ ───────────► │  src/content/albums/**   │
│  (Rust core +       │    sync      │  the content tree         │
│   Tauri shell)      │ ◄──────────► │                           │
└────────────────────┘              └────────────┬──────────────┘
                                                  │ read at request time
┌────────────────────┐    writes    ┌────────────▼──────────────┐
│  Admin panel        │ ───────────► │  Astro gallery (SSR)      │
│  (Express, local    │              │  the public site          │
│   only, :4444)      │              │  (:4321 dev, PM2 prod)    │
└────────────────────┘              └───────────────────────────┘
```

**The contract is the content tree.** An album is a folder under
`src/content/albums/` holding an `index.md` (frontmatter per the zod schema in
`src/content/config.ts`), optional `body.md` prose, the photo files, and a
server-owned `.meta/` directory (thumbnails cache, proofing submissions). Both
the admin panel and the desktop app produce exactly this shape; the gallery
consumes it. `gpp-core` asserts the frontmatter field set against the site's
schema in a test, so drift fails CI instead of producing albums the site
refuses to render.

## The parts

| Part | Where | Runs | Purpose |
|---|---|---|---|
| Gallery | `src/` | Node SSR (Astro, `output: 'server'`) | The public site: albums, lightbox, search, tags, proofing, downloads — with access control on every media route |
| Admin | `admin/` | Express on loopback, never deployed | Local CMS: album CRUD, uploads, reordering, proofing review, script runner |
| Core | `desktop/crates/gpp-core` | Anywhere Rust runs | Catalog (SQLite), import, develop, publish, sync — all the logic, no GUI |
| CLI | `desktop/crates/gpp-cli` | Terminal | Drives the whole core headless; the way to test without a GUI |
| C ABI | `desktop/crates/gpp-ffi` | Linked into non-Rust apps | One JSON call (`gpp_call`) covering every `Session` method; panics never cross |
| Shell | `desktop/app` | Tauri v2 (macOS first, iOS-ready) | One `#[tauri::command]` per UI action; UI embedded at compile time |

**`Session` is the application-level API** — roughly one method per thing a UI
can do. The shell, the CLI and the C ABI are thin wrappers over it. If logic is
creeping into any of them, it belongs in the core instead. A method missing
from `Session` is a capability no platform will ever have.

## Data flows worth knowing

**Photographer's day-to-day (desktop):** import a card → the core copies
outside folders in, hashes, reads EXIF, builds thumbnails in parallel → cull
with ratings/flags → develop (non-destructive, see [core.md](core.md)) →
publish into the content tree → sync to the server per album, in a direction
each album chose.

**Photographer's day-to-day (web admin):** upload into an album folder →
gallery picks it up at request time → thumbnails generated on first request,
cached under the album's `.meta/thumbnails/`.

**A visitor:** every page is SSR. Access is resolved per request from a signed
cookie or a share token ([security.md](security.md)); media routes re-check it
independently, so a protected album's pixels are unreachable even with a
guessed URL.

**Deploy:** `npm run deploy` builds the site and rsyncs it. The local content
tree is the source of truth (`--delete`), *except* `.meta/` which is
server-owned — proofing submissions are written by visitors on the server and
only ever pulled.

## Why the desktop core is Rust, and portable

Three constraints, stated once here and enforced everywhere:

1. **iPadOS is a target.** No `std::process`, no shelling out, nothing
   platform-specific outside a trait (`sync::RemoteTransport`,
   `media::RawDecoder`). The core compiles and its tests run on any platform —
   including the Linux container this repo is usually developed in.
2. **The gallery stays as-is.** The desktop app feeds the same content tree;
   nothing downstream changes when the core gains a feature.
3. **Licences stay permissive** (MIT/Apache/BSD/CC0), checked per shipping
   target. This is why HEIF decoding is pure Rust (`heif-oxide`) rather than
   libheif (LGPL), and why the C header is hand-written rather than generated
   (cbindgen and UniFFI are MPL).

## Where state lives

| State | Location | Rebuildable? |
|---|---|---|
| Photographs | the library folder (desktop) / `src/content/albums` (site) | **No — the only primary data** |
| Catalog | `<library>/.gpp/catalog.db` (SQLite, WAL) | Yes, by re-importing |
| Edits | `edits` table (JSON op stacks) | No — but tiny, and synced state |
| Renders/thumbs | `<library>/.gpp/thumbs/` (content-addressed) | Yes, on demand |
| Gallery thumbs | `<album>/.meta/thumbnails/` | Yes, on first request |
| Gallery metadata cache | `<album>/.meta/index.json` (size+mtime keyed) | Yes |
| Sync baselines | `sync_state` + `published_files` tables | Partially — see [sync.md](sync.md) |
| Proofing submissions | `<album>/.meta/proofing/*.json` on the **server** | No — server-owned, pull-only |
