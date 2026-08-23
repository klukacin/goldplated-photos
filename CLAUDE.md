# CLAUDE.md

> **What is this file?**
> This file provides comprehensive project context for [Claude Code](https://claude.ai/code) (Anthropic's AI coding assistant). When you open this project with Claude Code, it automatically reads this file to understand the codebase architecture, patterns, and conventions.

## For Contributors

**This project was built by Kristijan Lukacin with Claude AI assistance.** We encourage contributors to use Claude Code for development:

1. Install [Claude Code](https://claude.ai/code) CLI
2. Navigate to the project directory
3. Run `claude` to start an AI-assisted session
4. Claude will automatically read this file for context

See [CONTRIBUTING.md](CONTRIBUTING.md) for full contribution guidelines.

---

## Platform Compatibility

This project was developed on macOS but supports **macOS**, **Linux**, and **Windows**.

### Compatibility Matrix

| Feature | macOS | Linux | Windows | Windows + WSL |
|---------|-------|-------|---------|---------------|
| Dev server (`npm run dev`) | ✅ | ✅ | ✅ | ✅ |
| Admin panel (`npm run admin`) | ✅ | ✅ | ✅ | ✅ |
| Build (`npm run build`) | ✅ | ✅ | ✅ | ✅ |
| Background scripts (`dev:bg`, `admin:bg`) | ✅ | ✅ | ❌ | ✅ |
| Deploy script (`npm run deploy`) | ✅ | ✅ | ❌ | ✅ |

### Windows Setup

#### Option 1: WSL2 (Recommended - Full Compatibility)

WSL2 provides a native Linux environment inside Windows with full compatibility.

**Install WSL2:**
```powershell
# Run in PowerShell as Administrator
wsl --install
```

**After restart, set up Ubuntu:**
```bash
# Update packages
sudo apt update && sudo apt upgrade -y

# Install Node.js (via nvm - recommended)
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
source ~/.bashrc
nvm install 20
nvm use 20

# Install ffmpeg (for video metadata)
sudo apt install ffmpeg -y

# Clone and run project
git clone <repository-url>
cd Photo-gallery
npm install
npm run dev
```

**Access from Windows:** Files are at `\\wsl$\Ubuntu\home\<username>\Photo-gallery`

#### Option 2: Native Windows (Limited - No Deploy)

For development only (no bash scripts or deployment).

**Prerequisites:**
1. **Node.js 20+**: Download from [nodejs.org](https://nodejs.org/)
2. **Git**: Download from [git-scm.com](https://git-scm.com/)
3. **FFmpeg** (optional, for video metadata):
   - Download from [ffmpeg.org/download.html](https://ffmpeg.org/download.html)
   - Add to PATH: `setx PATH "%PATH%;C:\path\to\ffmpeg\bin"`

**Setup:**
```powershell
# Clone repository
git clone <repository-url>
cd Photo-gallery

# Install dependencies
npm install

# Start dev server (foreground only)
npm run dev

# In another terminal, start admin panel
npm run admin
```

**Limitations on native Windows:**
- ❌ `npm run dev:bg` / `npm run admin:bg` (background scripts)
- ❌ `npm run stop:dev` / `npm run stop:admin`
- ❌ `npm run deploy` (requires bash + rsync)
- ❌ `npm run deploy:parallel`

**Workaround for background servers:** Open two terminal windows and run `npm run dev` and `npm run admin` in foreground.

#### Option 3: Git Bash (Partial Compatibility)

Git Bash provides a bash shell on Windows but with limitations.

**Install:** Comes with [Git for Windows](https://git-scm.com/)

**Works:**
- Basic bash scripts
- `npm run dev`, `npm run admin`, `npm run build`

**Does NOT work:**
- `rsync` (not included) - deploy will fail
- Some Unix commands may behave differently

### Linux Setup

Linux has native compatibility. Install dependencies:

**Ubuntu/Debian:**
```bash
# Install Node.js (via nvm)
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash
source ~/.bashrc
nvm install 20

# Install ffmpeg (for video metadata)
sudo apt install ffmpeg -y

# Install rsync (for deployment, usually pre-installed)
sudo apt install rsync -y
```

**Fedora/RHEL:**
```bash
# Install Node.js
sudo dnf install nodejs -y

# Install ffmpeg and rsync
sudo dnf install ffmpeg rsync -y
```

**Arch Linux:**
```bash
# Install Node.js, ffmpeg, rsync
sudo pacman -S nodejs npm ffmpeg rsync
```

### Video Metadata Support

The `/api/video-info` endpoint requires **FFmpeg** (specifically `ffprobe`) for extracting video metadata.

**Verify installation:**
```bash
ffprobe -version
```

If FFmpeg is not installed, video info features will fail gracefully but videos will still play.

---

## Development Commands

```bash
# Development
npm run dev          # Start dev server (foreground, port 4321)
npm run dev:bg       # Start dev server in background
npm run stop:dev     # Stop background dev server

# Admin Panel (local content management)
npm run admin        # Start admin panel (foreground, port 4444)
npm run admin:bg     # Start admin panel in background
npm run stop:admin   # Stop background admin panel

# Build & Deploy
npm run build        # Build production site to ./dist/
npm run preview      # Preview production build locally
npm run deploy       # Full deployment to production server

# Maintenance
npm run update       # Normalize album structure (auto-create index.md)

# Quality
npm run check        # TypeScript check (astro check) - must pass
npm test             # Unit tests (vitest) - access control, rate limiting
```

### Recommended Development Setup

```bash
# Start both servers
npm run dev:bg
npm run admin:bg

# Work in browser
# Admin: http://localhost:4444
# Gallery: http://localhost:4321

# Stop when done
npm run stop:dev
npm run stop:admin
```

## Site Structure

The site has three main sections:

| Route | Page | Description |
|-------|------|-------------|
| `/` | Landing Page | Background image with shutter button, navigates to /home |
| `/home` | Digital Home | Hero slider, intro text, content cards |
| `/photos/*` | Photo Gallery | Album browser with all photo features |

## Application Overview

### Pages (Screens)

| Route | File | Description |
|-------|------|-------------|
| `/` | `src/pages/index.astro` | Landing page - full-screen background with shutter button |
| `/home` | `src/pages/home.astro` | Digital home - hero slider, intro text, content cards |
| `/photos` | `src/pages/photos/index.astro` | Gallery root - album list with Public/Locked toggle and live search box |
| `/photos/*` | `src/pages/photos/[...path].astro` | Album/Collection view - dynamic route for all albums |
| `/photos/tags/[tag]` | `src/pages/photos/tags/[tag].astro` | Prerendered tag page - albums with the tag (locked = no cover; hidden or locked *by an ancestor* counts as hidden or locked) |
| `/photos/search` | `src/pages/photos/search.astro` | SSR search - albums (title/description/tags) + photos (filename/camera/EXIF date); PUBLIC content only |

### Components

| Component | File | Description |
|-----------|------|-------------|
| PhotoGrid | `src/components/PhotoGrid.astro` | Photo/video grid, lightbox, EXIF/video info, keyboard nav, sorting, inline video players |
| AlbumGrid | `src/components/AlbumGrid.astro` | Sub-album grid with cover photo thumbnails |
| Breadcrumbs | `src/components/Breadcrumbs.astro` | Hierarchical navigation path |
| SEO | `src/components/SEO.astro` | Open Graph and Twitter Card meta tags for social sharing |
| Footer | `src/components/Footer.astro` | Site footer with email contact and copyright |

### API Endpoints

| Endpoint | File | Description |
|----------|------|-------------|
| `/api/thumbnail` | `src/pages/api/thumbnail.ts` | Generate/serve cached thumbnails (small/medium/large), access-checked |
| `/api/exif` | `src/pages/api/exif.ts` | Extract EXIF metadata from photos, access-checked |
| `/api/video-info` | `src/pages/api/video-info.ts` | Extract video metadata via ffprobe, access-checked |
| `/api/watermark` | `src/pages/api/watermark.ts` | Watermarked JPEG for social sharing, access-checked |
| `/api/unlock` | `src/pages/api/unlock.ts` | SSR password verification, sets signed HttpOnly cookie |
| `/api/download-album` | `src/pages/api/download-album.ts` | Streamed ZIP of album photos (cookie or X-Album-Token; checks ancestors; requires `allowDownload`) |
| `/api/proofing` | `src/pages/api/proofing.ts` | Store a client proofing submission (requires `proofing: true` + album access; rate-limited; validated against the album's photo list; saved to `.meta/proofing/`) |

**All media routes enforce album access** via `src/lib/access.ts` — originals (`/albums/*`), thumbnails, EXIF, video info, watermark and ZIP download all deny protected content without a valid signed cookie or share token.

### PhotoGrid Views & States

| View | Description |
|------|-------------|
| Grid view | Square thumbnails in responsive grid |
| Masonry view | Pinterest-style variable-height layout |
| Single-column view | Full-width images with original aspect ratios |
| Lightbox | PhotoSwipe full-screen image viewer |
| EXIF/Video Info overlay | Metadata popup (camera settings for photos, duration/codec for videos) |
| Video inline player | Native HTML5 video with controls |
| Video error state | Error message with filename and download button |

## Architecture Overview

### Rendering Mode
- **Server-side rendering** (`output: 'server'`) with Node.js standalone adapter
- API routes run server-side for password protection, EXIF extraction, and thumbnail generation
- Static pages are pre-rendered using `export const prerender = true`
- **Protected albums use SSR** (`prerender = false`) - content only rendered after access verification

### Content Collections

**Albums** (`src/content/albums/`):
```
src/content/albums/
  └── {year}/
      └── {category}/
          └── {album-name}/
              ├── index.md          # Album metadata (required)
              └── *.jpg/png/gif     # Photo files
```

Each `index.md` contains frontmatter defining album properties (title, password, thumbnail, style, etc.) per the schema in `src/content/config.ts`.

**Home Content** (`src/content/home/`):
```
src/content/home/
  ├── intro.md              # Introduction paragraph
  └── cards/
      ├── weddings.md       # Content cards with type, title,
      ├── people.md         # image, imagePosition (left/right),
      ├── events.md         # link, and order fields
      └── ...
```

### Routing System

**Landing Page:** `src/pages/index.astro`
- Full-screen background image
- Shutter button with sound effect (`/sounds/shutter.mp3`)
- Navigates to `/home` on click

**Digital Home:** `src/pages/home.astro`
- Hero slider with auto-rotating images from `/home/hero/`
- Intro text block from `intro.md`
- Content cards with alternating image positions
- **Accessibility**: Skip link, keyboard nav, ARIA carousel, pause control, live regions
- **Design**: Minimalist Modern (Syne + DM Sans typography, black/white palette)

**Photo Gallery Root:** `src/pages/photos/index.astro`
- Displays top-level albums with breadcrumb navigation (Home / Photos)
- **Public/Locked Toggle:** Filters albums by password protection status
  - Public: Albums without passwords (default view)
  - Locked: Password-protected albums
  - Toggle preference persisted to localStorage
- Albums sorted by `order` field, then alphabetically

**Photo Gallery:** `src/pages/photos/[...path].astro`
- Dynamic route handling all album paths under `/photos/`
- **Uses SSR** (`prerender = false`) for server-side access control
- Fetches album metadata from Content Collection
- **Access verification:** Checks `album-access` cookie before rendering content
- If password-protected and NOT unlocked: Shows inline password form (no image URLs in source)
- If authorized: Renders full content
- Distinguishes between collections (folders) and albums (photos):
  - **Collections** (`isCollection: true`): Renders `<AlbumGrid>` of sub-albums
  - **Albums** (`isCollection: false`): Renders `<PhotoGrid>` with photos

Photos are discovered by scanning the album directory for image files (see `getPhotosForAlbum()` in `src/lib/albums.ts`).

### Image Serving & Thumbnails

**Original images:** `src/pages/albums/[...path].ts` serves files from `src/content/albums/`
- Security: Prevents path traversal (`..` and leading `/` blocked)
- Caching: `Cache-Control: public, max-age=31536000, immutable`

**Thumbnails:** `src/pages/api/thumbnail.ts` generates optimized thumbnails using Sharp
- Three sizes: `small` (400px), `medium` (1200px), `large` (1920px)
- Cached in `.meta/thumbnails/{size}/` using MD5 hash filenames
- **To regenerate:** Delete `.meta/thumbnails` directory (ignored by git)
- PhotoGrid component automatically requests thumbnails via `getThumbnailUrl()` helper

**EXIF Orientation:** Sharp's `.rotate()` is applied during thumbnail generation to auto-rotate images based on EXIF orientation metadata.
- **IMPORTANT:** Always use thumbnails for display (not original images) to ensure correct orientation
- Original images may display rotated wrong because browsers don't consistently respect EXIF orientation
- Lightbox always uses large thumbnails (1920px) for this reason

### Album Cover Photos

Albums display cover photos in the grid using this priority:
1. **Manual selection:** Set `thumbnail: "filename.jpg"` in album's `index.md`
2. **Auto-fallback:** First photo in the album
3. **Empty albums:** Show generic icon (📁 for collections, 🖼️ for albums)

### Photo Sorting

Albums support sort options via dropdown (persisted to localStorage):
- Album Order (custom — the admin's drag-drop `photoOrder`, applied server-side when `sort: custom`)
- Name (A-Z / Z-A)
- Date taken / EXIF date (Oldest / Newest)
- File size (Smallest / Largest)

### Tags & Search

- Tag pills on album pages link to prerendered `/photos/tags/<tag>` pages (non-hidden albums; locked ones render title + lock placeholder only).
- The `/photos` search box filters album cards live (via `data-search` on cards); submitting goes to `/photos/search?q=`.
- `/photos/search` matches albums by title/description/tags and photos by filename, camera and EXIF date (ISO `2025-06-14` or `14.06.2025`) using the per-album metadata cache. Only fully public chains are searchable — hidden/locked albums and their content never appear. Pure matching logic: `src/lib/search-core.ts` (unit-tested).

### Slideshow

`style: slideshow` renders the grid as `grid` and auto-opens an auto-advancing lightbox (interval: `siteConfig.features.slideshowIntervalMs`, default 5 s; disabled under `prefers-reduced-motion`; `?photo=` deep links take priority). Every lightbox has a play/pause toolbar button (`P` key); manual navigation or tapping pauses.

### Client Proofing

Enable per album with `proofing: true` (checkbox in admin Settings). Visitors get a heart on every photo (grid + lightbox, `L` key), selections persist in localStorage per album, and a bottom bar opens a review panel (per-photo comments + optional name) that POSTs to `/api/proofing`. Submissions are JSON files in `<album>/.meta/proofing/`, browsable in the admin's Proofing tab (thumbnails, comments, copy-list, CSV export, delete) with a ♥ badge in the album tree.

### Access Control (src/lib/access.ts + access-core.ts)

**SECURITY:** Protected albums use Server-Side Rendering (SSR) — image URLs are NOT exposed in page source until access is verified, and every media route re-checks access server-side.

**Album access types:**
| Type | Frontmatter | Behavior |
|------|-------------|----------|
| Public | (none) | Freely accessible |
| Password-protected | `password: "..."` | Password form; unlock sets signed cookie |
| Link-share | `shareToken: "<random>"` | Reachable ONLY via secret link `?token=<shareToken>` |

An album can have both — the share link then skips the password form.

**Tokens:**
- `token` (required) — internal album id stored in the access cookie. Grants nothing by itself.
- `shareToken` (optional) — random secret, generated in the admin panel or via `node scripts/add-share-token.mjs <album-path>`. The ONLY value accepted from `?token=`/`X-Album-Token`.
- `allowAnonymous` — DEPRECATED, ignored.

**Signed cookie:**
- `album-access` cookie value is `base64url(json).hmac` signed with `ACCESS_SECRET` from `.env` (min 16 chars). Without it an ephemeral secret is used (sessions reset on restart).
- Cookie flags: `httpOnly`, `secure` (prod), `sameSite: strict`, 24h expiry. Forged/unsigned cookies are rejected.

**Unlock flow:**
1. User submits password via form POST to `/api/unlock`
2. Server validates with timing-safe comparison + rate limiting (10 attempts/15 min per real client IP — X-Forwarded-For aware behind the proxy)
3. On success: unlocks the album + cascades to password-less descendants, sets signed cookie, redirects

**Access inheritance:**
- A locked ancestor blocks descendants until unlocked; an unlocked album grants its descendants
- Share tokens of ancestors also grant descendants
- **A grant stops at the nearest lock.** A descendant with its own `password` or `shareToken` stays locked whether the ancestor was opened by cookie or by share link — so handing a client the collection's secret link does not hand them the separately locked albums inside it

**Single implementation:** `resolveAlbumAccess()` / `resolveFileAccess()` are used by the album page AND all media routes (`/albums/*`, thumbnail, exif, video-info, watermark, download-album). Never add a media route without calling them. Pure logic lives in `access-core.ts` (unit-tested in `tests/`).

Passwords are plaintext strings in frontmatter (simple protection, not cryptographically secure).

### EXIF Data

**API:** `src/pages/api/exif.ts` extracts EXIF using `exifr` library
- Client requests EXIF for specific photo URL
- Server reads file, extracts metadata, returns JSON
- **Caching:** PhotoGrid maintains in-memory cache (`exifCache` Map) per session

**Display:** PhotoGrid component shows EXIF in overlay (press `i` key or click info button)

### PhotoSwipe Integration

`PhotoGrid.astro` initializes PhotoSwipe lightbox with:
- Full-screen photo viewing
- Touch gestures (swipe, pinch-to-zoom)
- Keyboard navigation (arrows, `i` for EXIF, `F` for fullscreen, Escape to close)
- Custom escape key handling: First press closes help, second closes EXIF, third closes lightbox
- Scroll-to-last-viewed-photo on lightbox close
- Real-time grid focus sync during navigation

### Custom Lightbox UI

The lightbox includes a custom toolbar (dynamically created in `uiRegister` event):

**UI Elements (top bar):**
- **Left:** Image counter with thumbnail preview (e.g., "1 / 24")
- **Right:** Size display, Original button (🔍+), Help (?), Close (×)

**Features:**
- **Original Mode (O key):** Load full-resolution original images instead of thumbnails
- **Zen Mode (H key):** Hide all UI elements for distraction-free viewing (hover reveals UI)
- **Shortcuts Help (? key):** Overlay showing all keyboard shortcuts

**CSS Note:** All custom lightbox UI styles use `:global()` in Astro because the elements are dynamically created and appended to PhotoSwipe's container (outside the component's scoped DOM).

### Keyboard Navigation

**Gallery view** (not in lightbox):
- Arrow keys: Navigate photos in left-to-right, top-to-bottom order
- Space/Enter: Open focused photo
- `i`: Toggle EXIF overlay
- Home/End: Jump to first/last photo
- Visual focus indicator (blue outline) via `.keyboard-focused` class

**Lightbox view:**
- Arrows: Navigate photos
- `i` / `I`: Toggle EXIF/video info overlay
- `F` / `f`: Toggle fullscreen mode
- `O` / `o`: Toggle original quality images
- `H` / `h`: Toggle zen mode (hide all UI)
- `?`: Toggle keyboard shortcuts help
- Escape: Close overlays in order (help → EXIF → lightbox)
- Event capture phase intercepts keys before PhotoSwipe handlers

### Mobile UX

**Desktop:** Hover icons (info, download) appear in top-right corner of photos

**Mobile (<768px):**
- Icons hidden
- Long-press (500ms) triggers context menu at touch position
- Haptic feedback on long-press (`navigator.vibrate(50)`)
- Menu options: Photo Info, Download
- **Swipe up/down:** Close lightbox

### Masonry Layout

CSS Grid with `grid-template-columns: repeat(3, 1fr)` ensures true left-to-right ordering.

- Desktop: 3 columns
- Mobile: 2 columns

### Social Sharing

All pages include Open Graph and Twitter Card meta tags for rich social sharing previews.

**Meta tags included:**
- Open Graph: `og:title`, `og:description`, `og:image`, `og:url`, `og:type`, `og:site_name`
- Twitter: `twitter:card`, `twitter:title`, `twitter:description`, `twitter:image`, `twitter:creator`

**OG Image selection by page:**
| Page | OG Image |
|------|----------|
| Landing (`/`) | Landing background image |
| Home (`/home`) | First hero slider image |
| Gallery (`/photos`) | First public album's cover photo |
| Album (`/photos/*`) | Album's cover photo |
| Collection (`/photos/*`) | First sub-album's cover photo |
| Protected (no access) | Generic locked image, `noindex` meta |

**Individual Photo Sharing:**
Share specific photos using URL parameter: `/photos/album-path?photo=filename.jpg`
- SSR generates meta tags with that photo as `og:image`
- Lightbox auto-opens to the shared photo
- Works for public albums only

**Configuration:**
Edit `src/config.ts` to change site name, URL, Twitter handle, and default OG image.
Environment variables (`.env`) can override site URL for different environments.

### Design System (Layout.astro)

**Typography:**
- Headings: `Syne` (Google Font) - geometric, variable weight 500-800
- Body: `DM Sans` (Google Font) - clean sans-serif
- Signature: `Satisfy` (Google Font) - elegant script for "by Kristijan Lukačin"
- Fluid scale using `clamp()`: `--text-xs` to `--text-4xl`
- CSS variable: `--font-signature: 'Satisfy', cursive`

**Color Tokens (high contrast, WCAG AA):**
- `--color-text: #0a0a0a` (21:1 contrast on white)
- `--color-text-secondary: #404040` (9.5:1 contrast)
- `--color-text-muted: #666666` (5.7:1 contrast)
- `--color-focus: #2563eb` (blue focus ring)

**Spacing Scale:**
- `--space-xs` (0.5rem) to `--space-3xl` (8rem)
- Container widths: `--container-narrow` (700px), `--container-max` (1000px), `--container-wide` (1200px)

**Accessibility Utilities:**
- `.sr-only` - Screen reader only content
- `.skip-link` - Skip to main content link
- `:focus-visible` - 3px blue outline with 2px offset
- `@media (prefers-reduced-motion: reduce)` - Disables all animations
- `@media (forced-colors: active)` - High contrast mode support

### Home Page Accessibility (home.astro)

**Carousel (AccessibleSlider class):**
- ARIA: `role="region"`, `aria-roledescription="carousel"`, `aria-label`
- Slides: `role="group"`, `aria-roledescription="slide"`, `aria-hidden` state
- Live region announces slide changes to screen readers
- Keyboard: Arrow keys navigate, Space/Enter toggles pause
- Pause button with `aria-pressed` state
- Auto-play disabled when `prefers-reduced-motion: reduce`

**Card Layout:**
- Uses CSS Grid `order` property for alternating image positions (no RTL hack)
- Entry animations via Intersection Observer (respects reduced motion)

**Touch Targets:**
- All interactive elements ≥44x44px (WCAG requirement)
- Slider dots use `::before` pseudo-element for expanded tap area

## Admin Panel

A local-only web-based CMS for content management. **Never deployed to production.**

**SECURITY:** the server binds to loopback, but every page the photographer visits can reach loopback too, and `cors()` does not stop that — it decides who may *read* a response, long after the handler has run. A fetch-metadata guard (`Sec-Fetch-Site` / `Origin`, in `admin/server.js` before every route) rejects anything sent from another origin, which is what keeps a stray `<img src="http://localhost:4444/api/tools/run/deploy">` on someone else's website from pushing the site live. Non-browser clients (curl) send neither header and still work. `ADMIN_PORT` overrides the 4444 default.

**Location:** `admin/` directory
**Server:** Express.js on port 4444
**Frontend:** Vanilla JS + CodeMirror markdown editor

### Tabs

| Tab | Purpose |
|-----|---------|
| **Albums** | Create/edit/rename/move albums, reorder siblings (↑↓ in tree), upload photos/videos with progress, drag-drop photo reordering (persists `photoOrder` + sets `sort: custom`), multi-select bulk delete/move/set-cover, click-to-preview with EXIF, share-token management |
| **Home** | Edit landing background, hero slider, intro text, content cards (drag to reorder) |
| **Tools** | Thumbnail cache management, script runner (build/deploy/maintenance with live output), quick links |

Unsaved album changes prompt before switching albums or closing the tab. CodeMirror is vendored locally (`admin/vendor/`), so the admin works offline.

### Admin API Endpoints

| Endpoint | Purpose |
|----------|---------|
| `/api/albums`, `/api/albums/*path` | Album CRUD (PUT merges frontmatter; `null` clears a field) |
| `/api/album-rename/*path` | Rename/move an album folder |
| `/api/albums-reorder` | Persist sibling album order (`order` fields) |
| `/api/photos/*path` | Photo upload/delete |
| `/api/photo-order/*path` | Save drag-drop photo order (`photoOrder`) |
| `/api/photo-bulk/delete\|move/*path` | Bulk photo operations |
| `/api/photo-exif/*path` | EXIF for the admin preview |
| `/api/videos/*path` | Video upload/delete |
| `/api/home/intro` | Intro text |
| `/api/home/cards` | Content cards CRUD + reorder |
| `/api/assets/hero`, `/api/assets/cards` | Asset management |
| `/api/cache/stats`, `/api/cache/thumbnails` | Cache management |
| `/api/tools/scripts`, `/api/tools/run/:id` | Whitelisted script runner (SSE output, one at a time) |

### Data Flow

```
Admin Panel (browser) → Admin API (:4444) → File System → Dev Server (:4321)
```

Changes made in admin are saved directly to `src/content/` and `public/`, then auto-reloaded by dev server.

## Desktop App (`desktop/`)

A native photo workflow — import, cull, develop, publish, sync — sitting in front of the same gallery the admin panel edits. Written in Rust so it can run where a browser cannot: macOS today, iPadOS and Windows on the same code.

`desktop/ARCHITECTURE.md` is the long form. This is what you need before touching it.

### The shape

```
gpp-core  ──  all the logic: catalog, import, albums, develop, publish, sync
   │              no GUI, no async runtime, no shelling out
   ├── gpp-cli       a command-line driver — the way to test without a GUI
   ├── gpp-ffi       a C ABI: one JSON call, for clients that are not Rust
   └── gpp-desktop   a Tauri v2 shell: one #[tauri::command] per UI action
```

`Session` is the application-level API — roughly one method per thing the UI can do. The shell, the CLI and the C ABI are all thin wrappers over it; if logic is creeping into any of them, it belongs in the core instead.

`gpp-ffi` is what makes a *native* iPad or Android client possible rather than only a webview one: four `extern "C"` functions, `gpp_call(session, method, args_json)` covering the whole of `Session`, replies always `{"ok": …}` or `{"error": …, "kind": …}`, and no panic ever allowed to unwind into C. Its header is hand-written and checked in — cbindgen and UniFFI are both MPL-2.0, which the licence policy does not allow even at build time. See `desktop/crates/gpp-ffi/README.md`.

**The portability contract** (stated in `gpp-core/src/lib.rs`, and it is load-bearing): no GUI dependencies, no spawning external processes, anything platform-specific behind a trait the shell implements — `sync::RemoteTransport`, `media::RawDecoder`. Breaking it is how the iPad target quietly dies.

### The library on disk

A library is a folder of photos the user already has. The app never moves them; it writes a catalog beside them:

```
<library>/
  2026/weddings/ana-ivan/*.jpg     the photographer's own folders, untouched
  .gpp/catalog.db                  SQLite: photos, albums, edits, sync state
  .gpp/thumbs/<shard>/<key>_*.jpg  content-addressed derived images
```

Importing a folder from **outside** the library copies it in, because the catalog addresses photos by their path under the root and cannot point anywhere else.

### Develop is non-destructive

An adjustment is a row in `edits`, never a write to the original. What identifies a rendered image is a **render key**: the photo's content hash when there are no edits, otherwise `blake3(content_hash + stack_json)`. Change an adjustment and the key changes, so the grid asks for a thumbnail that does not exist yet and gets a freshly rendered one; reset it and the key returns to the original's, which is still cached. Publishing ships the developed pixels.

Geometry (rotate, flip, crop) applies before tone, so a crop rectangle means the same thing regardless of exposure.

**Where order matters, and where it cannot.** `apply` runs two passes: geometry in stack order, then tone in stack order, purely per pixel. So a tone op's position relative to a geometry op is irrelevant — measured, not assumed (`tests/geometry_order.rs`). Within tone, order matters, as in any developer: exposure-then-contrast is not contrast-then-exposure. Within geometry it matters too, and that is why the turns and mirrors are **not** edited where they lie in the stack.

**Orientation is stored canonically**, as one left-to-right mirror followed by quarter turns — the eight ways a rectangle can be set down. A button composes onto the *outside* of that framing and the whole thing is written back, so a top-to-bottom flip is stored as a mirror plus a half turn, and `flip-vertical` is never written. Editing the ops in place instead gave two defects that were three presses away in the panel: "rotate right" on a flipped frame turned the photograph left, and pressing a flip again to undo it mirrored the wrong axis once a turn sat between them. Anything reading orientation must fold the whole op list (as `renderGeometry` does), never look for a particular op. The crop is held last and its rectangle is carried along by each press, so a frame drawn on the picture keeps framing the same part of it.

**Nothing ever writes to the original.** The only writes in the core are: the render cache and thumbnails (`.gpp/`), the published tree (`dest_root`), and copying a photo *into* the library on import. `ensure_rendered` returns the original's own path when the stack is empty — no copy, no cache entry — so an untouched photo costs nothing and a Reset is instant. **A pull adds photos to the library but never overwrites one that is already there**: the sync plan compares the *published* copy against the remote, and the published copy holds developed pixels, so the library original was never part of that comparison. A photo the server disagrees on is reported in `PullOutcome.kept_originals` rather than replaced.

### Publish and sync are different things

- **Publish** writes the gallery's content tree — `index.md` frontmatter plus the photo files — into `src/content/albums`. It only removes files this library published before, so nothing the admin panel or another tool put there is ever touched.
- **Sync** moves that tree to a server. Per album, in a direction that album chose (`push`, `pull`, `both`, or untracked). **An album nobody tracks is never touched on either side** — not pushed, not pulled, not deleted.

Three manifests decide every file: what is local, what was last synced, what the remote holds. Both sides changed since the baseline is a conflict, and a conflict is reported, never resolved by guessing. Deleting on the server needs `allow_deletes` — the UI asks first and names the files.

Two transports, same trait: a folder (network share, external drive) or the gallery's own HTTP endpoints. `.meta/` is server-owned — the proofing submissions live there — and is excluded from every manifest in both directions.

**A subscription follows its album.** Renaming or moving an album carries its subscription and its sub-albums' subscriptions; deleting an album drops its subscription. Nothing on the remote moves, though — a tracked album that was already pushed stays on the server under the old path too, and the next sync publishes it under the new one, so the server holds both until someone deletes the old copy with `allow_deletes`.

**One album's failure never stops the batch.** "Sync all tracked" reports each album's outcome separately, a failed album included, and carries on with the rest — the same way `apply` already treats a single failing file.

### Sync over HTTP

`/api/sync/manifest` and `/api/sync/file`, guarded by `SYNC_TOKEN` (min 16 chars). **Unset, they answer 503 rather than opening.** Paths are validated before touching disk; uploads are verified against `X-Content-Blake3` and written through a temp file. The server caches hashes by `(size, mtime)` — without it a sync re-hashed the whole library in JavaScript at 32 MB/s, which cost 40 s on every push. See `desktop/UPLOAD-TRANSPORT.md` for the measurements and why the transport is what it is.

### Commands

```bash
cd desktop
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings   # must be clean
cargo run -p gpp-cli -- --help                          # drive the core headless

cd desktop/app/src-tauri && cargo build                 # the shell
node desktop/app/check-shell.mjs                        # contract checks, see below
```

`npm run check:licences` holds every Rust dependency to MIT / Apache-2.0 / BSD / CC0, per target.

**Do not share one `CARGO_TARGET_DIR` between git worktrees.** It looks like a way to save disk and it costs correctness: a build of one worktree's sources can be served artifacts built from another's, so a test passes or fails for reasons that are not in the tree you are reading. It has already produced both a phantom failure and a phantom extra dozen tests. Give each worktree its own target directory, or build them one at a time.

### The shell cannot be unit-tested, so it has contract checks

`desktop/app/check-shell.mjs` encodes defects that shipped and were only found by launching the app: an `invoke()` with no registered command, an element id that is not in the markup, a missing Tauri capability, a CSP without `ipc:`, a `[hidden]` rule a class can override, a `build.rs` that does not watch the UI files. **The UI is embedded into the binary at compile time — rebuild after every UI edit or you are testing the previous JavaScript.**

Two traps worth knowing before you write UI code:

- `window.confirm()` inside a Tauri webview on Linux returns `true` without asking. Use `askConfirm` from the dialog plugin; the contract check enforces it.
- Types crossing the IPC boundary are built in JavaScript and read by serde. A name serde does not recognise is dropped in silence — which is how the star filter and the album sidebar once filtered nothing. Multi-word fields need `rename_all = "camelCase"`, and `deny_unknown_fields` turns the next typo into an error.

### Running it without a screen

`desktop/app/run-headless.sh <library>` starts Xvfb, a window manager (without one, synthetic clicks land nowhere), a session bus and the XDG portal (without it the folder picker opens nothing and reports no error). Pass a library path to skip the picker. `GPP_DISPLAY` and `GPP_BUS` let several run at once.

## Deployment Workflow

**Workflow:** Develop locally with admin panel → `npm run deploy` → Production server

```
LOCAL                           PRODUCTION
┌─────────────┐                ┌─────────────┐
│ Admin :4444 │                │             │
│     ↓       │  npm deploy    │ Node.js +   │
│ Dev :4321   │ ──────────────→│ PM2         │
└─────────────┘                └─────────────┘
```

**Key Points:**
- Admin panel is LOCAL ONLY (never deployed)
- Local content is source of truth (rsync uses `--delete`)
- `npm run deploy` handles: build → fix paths → rsync → PM2 restart

### Deploy Script (scripts/deploy.sh)

Supports both sequential and parallel rsync deployment modes.

```bash
npm run deploy                    # Sequential sync (default, simple)
npm run deploy:parallel           # Parallel sync (5 workers, faster)
npm run deploy -- --checksum      # Force checksum comparison (slower but thorough)
npm run deploy -- --parallel --checksum  # Combine flags
```

**Sequential mode (default):** Single rsync for entire albums tree. Zero redundancy, simple.

**Parallel mode:** Splits by top-level directories (ws, neos, friends, etc.), runs up to 5 concurrent rsync processes. Each handles its complete subtree with `--delete`. Final cleanup pass removes stale remote items. ~2x faster on large syncs.

**Steps:**
1. Sanitize folder names (lowercase)
2. `npm run build` - Build production site
3. Fix paths for production server
4. Sync client, public, albums, server files
5. Create symlinks on server
6. Set private file permissions
7. `npm install --production` + `pm2 restart` on remote

### Configuration

Edit `.env` (copy from `.env.example`):
```bash
# Server connection
DEPLOY_REMOTE_USER="username"
DEPLOY_REMOTE_HOST="server.com"
DEPLOY_REMOTE_ROOT="/path/to/public_html"

# File permissions (adjust for your server setup)
DEPLOY_CHMOD_DIRS=775      # Directory permissions
DEPLOY_CHMOD_FILES=664     # File permissions
DEPLOY_CHMOD_PRIVATE=660   # Private files (index.md, .htaccess)
# DEPLOY_CHOWN=user:www-data  # Optional ownership
```

**Permission Presets:**

| Setup | Dirs | Files | Private |
|-------|------|-------|---------|
| Shared hosting (same user) | 775 | 664 | 660 |
| Apache/Nginx (www-data) | 755 | 644 | 640 |

### Marketing Website Deployment

The marketing website (`marketing_website/`) is a separate static site deployed to goldplated.photos.

**Note:** The `marketing_website/` folder is in `.gitignore` (separate from main repo).

**Deploy command:**
```bash
# Deploy marketing site (excludes macOS metadata files)
sshpass -p 'PASSWORD' rsync -avz --progress --exclude='._*' \
  -e "ssh -o StrictHostKeyChecking=no" \
  marketing_website/ goldplated@mirjam.nosco.hr:public_html/
```

**Fix permissions after deploy:**
```bash
sshpass -p 'PASSWORD' ssh goldplated@mirjam.nosco.hr \
  "find public_html -type d -exec chmod 775 {} \; && \
   find public_html -type f -exec chmod 664 {} \; && \
   chmod 660 public_html/.htaccess"
```

**Marketing site files:**
- `marketing_website/index.html` - Homepage
- `marketing_website/features.html` - Features page
- `marketing_website/get-started.html` - Quick start guide
- `marketing_website/about.html` - About page
- `marketing_website/.htaccess` - Apache config (HTTPS redirect, caching, security headers)
- `marketing_website/css/styles.css` - Styles
- `marketing_website/js/main.js` - JavaScript

**Important:** Always use `--exclude='._*'` to prevent macOS metadata files from being uploaded.

### Documentation Site Deployment

The documentation site (`docs/`) uses MkDocs and deploys to docs.goldplated.photos.

**Build and deploy:**
```bash
# Build MkDocs
cd docs
mkdocs build

# Deploy to server
sshpass -p 'PASSWORD' rsync -avz --progress \
  -e "ssh -o StrictHostKeyChecking=no" \
  site/ docs@mirjam.nosco.hr:public_html/
```

## Key Files

**Pages:**
- `src/pages/index.astro` - Landing page with shutter button
- `src/pages/home.astro` - Digital home with hero slider and cards
- `src/pages/photos/index.astro` - Photo gallery root (album list)
- `src/pages/photos/[...path].astro` - Dynamic album/collection pages

**Components:**
- `src/components/PhotoGrid.astro` - Photo display, lightbox, EXIF, keyboard nav, sorting
- `src/components/AlbumGrid.astro` - Sub-album grid with cover photos
- `src/components/SEO.astro` - Open Graph and Twitter Card meta tags for social sharing
- `src/components/Breadcrumbs.astro` - Hierarchical navigation
- `src/components/Footer.astro` - Site footer with email contact and copyright
- `src/layouts/Layout.astro` - Base layout wrapper

**API Routes:**
- `src/pages/albums/[...path].ts` - Serve original images (access-checked)
- `src/pages/api/thumbnail.ts` - Generate/serve cached thumbnails (access-checked)
- `src/pages/api/exif.ts` - Extract EXIF metadata (access-checked)
- `src/pages/api/video-info.ts` - Video metadata via ffprobe (access-checked)
- `src/pages/api/watermark.ts` - Watermarked share image (access-checked)
- `src/pages/api/unlock.ts` - SSR password verification (sets signed HttpOnly cookie)
- `src/pages/api/download-album.ts` - Create ZIP of album photos (access-checked incl. ancestors)

**Configuration:**
- `src/config.ts` - Centralized site configuration (URL, name, social defaults)
- `.env.example` - Environment variables template (SITE_URL, ACCESS_SECRET, deploy settings)

**Utilities:**
- `src/lib/access-core.ts` - Pure access-control logic (signed cookie, share tokens, chain resolution) — unit-tested
- `src/lib/access.ts` - Astro glue: resolveAlbumAccess/resolveFileAccess/setAccessCookie
- `src/lib/albums.ts` - Album/photo discovery, breadcrumbs, cover photos
- `src/lib/rate-limit.ts` - In-memory rate limiting (10 attempts / 15 minutes per IP)
- `src/lib/media-info.ts` - Builds the lightbox EXIF/video-info overlay markup. Every interpolated value is HTML-escaped: EXIF strings (Make, Model, LensModel) ride inside the image file, so a photo from a client or second shooter can carry markup in them — unit-tested

**Tests & CI:**
- `tests/*.test.ts` - Vitest unit tests (access control, rate limiting)
- `.github/workflows/ci.yml` - CI: astro check → vitest → syntax checks → build

**Static Assets:**
- `public/images/landing-bg.jpg` - Landing page background
- `public/sounds/shutter.mp3` - Shutter button sound
- `public/home/hero/*.jpg` - Hero slider images
- `public/home/cards/*.jpg` - Content card images

**Admin Panel:**
- `admin/server.js` - Express.js admin API server
- `admin/index.html` - Admin panel frontend (single page)
- `admin/js/` - Admin panel JavaScript modules

**Scripts:**
- `scripts/deploy.sh` - Production deployment script
- `scripts/start-dev.sh` / `stop-dev.sh` - Dev server background control
- `scripts/start-admin.sh` / `stop-admin.sh` - Admin server background control
- `scripts/update-albums.mjs` - Album structure normalization (keeps admin-authored body.md)
- `scripts/fix-server-paths.mjs` - Production path fixer (reads DEPLOY_REMOTE_ROOT from env)
- `scripts/add-share-token.mjs` - Generate/remove/list album share tokens (secret links)

## Important Patterns

### Album Folder Naming
**IMPORTANT:** Avoid dots (`.`) in album folder names, especially after numbers.
- ❌ `16.album-name` → Astro normalizes to `16album-name` (breaks file lookups)
- ✅ `16-album-name` → Works correctly

Use dashes (`-`) or underscores (`_`) as separators instead of dots.

### Adding Photos to Albums
1. Copy photos to `src/content/albums/{path}/`
2. Photos auto-discovered by file extension (.jpg, .jpeg, .png, .gif, .webp, .heic, .heif)
3. Thumbnails generated on first request, cached thereafter

### Setting Album Cover Photo
In album's `index.md`:
```yaml
---
title: "My Album"
thumbnail: "best-photo.jpg"  # Optional: specific cover photo
---
```

### Album Ordering and Visibility
Albums support `order` and `hidden` fields in frontmatter:
```yaml
---
title: "My Album"
order: 1          # Lower numbers appear first (optional)
hidden: true      # Only accessible via direct link (default: false)
---
```
- **Order:** Albums without an order value appear after ordered albums
- **Hidden:** Hidden albums don't appear in album listings but can still be accessed directly

### Adding Home Page Content Cards
Create markdown file in `src/content/home/cards/`:
```yaml
---
type: "card"
title: "Card Title"
image: "/home/cards/image.jpg"
imagePosition: "left"  # or "right"
link: "/photos/album-path"
order: 1
---

Card body text in markdown.
```

### Modifying Thumbnail Quality/Size
Edit `src/pages/api/thumbnail.ts`:
- Change `THUMBNAIL_SIZES` object for dimensions
- Modify Sharp `.jpeg({ quality: 85 })` for compression

### Event Handling Hierarchy
PhotoGrid uses event **capture phase** (`addEventListener(..., true)`) to intercept keyboard events before PhotoSwipe's bubble-phase handlers.

### Async Content Rendering
When rendering Content Collections in Astro components, pre-render with `Promise.all`:
```typescript
const renderedCards = await Promise.all(
  cards.map(async (card) => ({
    ...card,
    RenderedContent: (await card.render()).Content
  }))
);
```

## Troubleshooting

**Thumbnails not updating:** Delete `.meta/thumbnails` directory

**Password not working:** Check `album-access` cookie in browser DevTools (Application > Cookies); clear it to reset. Rate limiting may block after 10 failed attempts (15 min cooldown).

**Navigation jumping:** Position-based nav uses 80px tolerance for row detection; adjust in `getNextIndex()` if needed

**EXIF not showing:** Some photos lack EXIF data; check console for errors from exifr library

**EXIF not visible in fullscreen:** Overlay should be inside PhotoSwipe container (handled automatically)

**Sort not persisting:** Check localStorage for `photoGallery_sortOption` key

**/home page not rendering:** Ensure `export const prerender = true` is set in frontmatter

**Slider not auto-playing:** Check if user has `prefers-reduced-motion: reduce` enabled in OS settings

**Focus styles not visible:** Ensure `:focus-visible` is not overridden; check for `outline: none` on elements

**Fonts not loading:** Check network tab for Google Fonts; ensure preconnect hints are in Layout.astro `<head>`

**Broken thumbnails / "album not found":** Astro's glob loader normalizes folder names, removing dots after numbers. Folders like `16.album-name` become album ID `16album-name`, causing file system lookups to fail. **Fix:** Use dashes instead of dots after number prefixes: `16-album-name`

**Rate limited on password entry:** Wait 15 minutes or restart the server (rate limit is in-memory). Check `src/lib/rate-limit.ts` to adjust limits.

**Protected album showing content unexpectedly:** Clear `album-access` cookie. Verify `prerender = false` in `[...path].astro`. Check that access verification runs before content fetch in frontmatter.

**Download album failing on protected content:** Ensure `X-Album-Token` header is sent with the request. Check browser DevTools Network tab for 401 errors.

**Admin panel won't start:** Check if port 4444 is in use (`lsof -i :4444`). Run `npm run admin` in foreground to see errors.

**Admin changes not showing in gallery:** Ensure dev server is running (`npm run dev`). Hard refresh browser (Cmd+Shift+R).

**Deploy failing:** Check SSH key access to server. Verify `REMOTE_USER`, `REMOTE_HOST`, `REMOTE_ROOT` in `scripts/deploy.sh`. Ensure PM2 and Node.js are installed on server.

**403 Forbidden after deploy:** File permissions may be wrong (rsync can set 700). Fix with:
```bash
chmod -R 755 public_html && find public_html -type f -exec chmod 644 {} \;
```
Required permissions: directories `755`, files `644`, `.htaccess` files `644`.
