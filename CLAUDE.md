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
| `/photos` | `src/pages/photos/index.astro` | Gallery root - top-level album list |
| `/photos/*` | `src/pages/photos/[...path].astro` | Album/Collection view - dynamic route for all albums |

### Components

| Component | File | Description |
|-----------|------|-------------|
| PhotoGrid | `src/components/PhotoGrid.astro` | Photo/video grid, lightbox, EXIF/video info, keyboard nav, sorting, inline video players |
| AlbumGrid | `src/components/AlbumGrid.astro` | Sub-album grid with cover photo thumbnails |
| Breadcrumbs | `src/components/Breadcrumbs.astro` | Hierarchical navigation path |
| SEO | `src/components/SEO.astro` | Open Graph and Twitter Card meta tags for social sharing |
| PasswordProtection | `src/components/PasswordProtection.astro` | Password entry form (DEPRECATED - now inline in SSR) |
| Footer | `src/components/Footer.astro` | Site footer with email contact and copyright |

### API Endpoints

| Endpoint | File | Description |
|----------|------|-------------|
| `/api/thumbnail` | `src/pages/api/thumbnail.ts` | Generate/serve cached thumbnails (small/medium/large) |
| `/api/exif` | `src/pages/api/exif.ts` | Extract EXIF metadata from photos |
| `/api/video-info` | `src/pages/api/video-info.ts` | Extract video metadata via ffprobe |
| `/api/check-password` | `src/pages/api/check-password.ts` | Validate album passwords (legacy) |
| `/api/check-access` | `src/pages/api/check-access.ts` | Check if user has access to album (legacy) |
| `/api/unlock` | `src/pages/api/unlock.ts` | SSR password verification, sets HttpOnly cookie |
| `/api/download-album` | `src/pages/api/download-album.ts` | Create ZIP of album photos (requires X-Album-Token header) |

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

Albums support 6 sort options via dropdown (persisted to localStorage):
- Name (A-Z / Z-A)
- Date taken (Oldest / Newest) - **default: oldest first**
- File size (Smallest / Largest)

### Password Protection Flow (SSR)

**SECURITY:** Protected albums use Server-Side Rendering (SSR) - image URLs are NOT exposed in page source until access is verified.

**How it works:**
1. Album page uses `prerender = false` (SSR, not static)
2. Server checks `album-access` cookie for unlocked tokens
3. If NOT authorized: Only password form is rendered (no image URLs in source)
4. If authorized: Full album content is rendered with image URLs

**Unlock flow:**
1. User submits password via form POST to `/api/unlock`
2. Server validates with timing-safe comparison + rate limiting (10 attempts/15 min)
3. On success: Sets HttpOnly cookie with album token, redirects to album
4. Cookie flags: `httpOnly`, `secure` (prod), `sameSite: strict`, 24h expiry

**Access inheritance:**
- Parent album access grants access to child albums without passwords
- Tokens can be passed via `?token=` query parameter (share links)

**Security features:**
- Path traversal protection on all API endpoints
- Rate limiting prevents brute force attacks
- Timing-safe password comparison prevents timing attacks
- HttpOnly cookies prevent XSS token theft

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

**Location:** `admin/` directory
**Server:** Express.js on port 4444
**Frontend:** Vanilla JS + CodeMirror markdown editor

### Tabs

| Tab | Purpose |
|-----|---------|
| **Albums** | Create/edit albums, upload photos/videos, manage settings |
| **Home** | Edit landing background, hero slider, intro text, content cards |
| **Tools** | Thumbnail cache management, quick links |

### Admin API Endpoints

| Endpoint | Purpose |
|----------|---------|
| `/api/albums`, `/api/albums/*path` | Album CRUD |
| `/api/photos/*path` | Photo upload/delete |
| `/api/videos/*path` | Video upload/delete |
| `/api/home/intro` | Intro text |
| `/api/home/cards` | Content cards CRUD |
| `/api/assets/hero`, `/api/assets/cards` | Asset management |
| `/api/cache/stats`, `/api/cache/thumbnails` | Cache management |

### Data Flow

```
Admin Panel (browser) → Admin API (:4444) → File System → Dev Server (:4321)
```

Changes made in admin are saved directly to `src/content/` and `public/`, then auto-reloaded by dev server.

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

1. `npm run build` - Build production site
2. Fix paths for production server
3. rsync files to remote (with `--delete` for albums)
4. Create symlinks on server
5. `npm install --production` on remote
6. `pm2 restart` on remote

### Configuration

Edit `scripts/deploy.sh`:
```bash
REMOTE_USER="username"
REMOTE_HOST="server.com"
REMOTE_ROOT="/path/to/public_html"
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
- `src/components/PasswordProtection.astro` - Password entry form
- `src/components/Breadcrumbs.astro` - Hierarchical navigation
- `src/components/Footer.astro` - Site footer with email contact and copyright
- `src/layouts/Layout.astro` - Base layout wrapper

**API Routes:**
- `src/pages/albums/[...path].ts` - Serve original images
- `src/pages/api/thumbnail.ts` - Generate/serve cached thumbnails
- `src/pages/api/exif.ts` - Extract EXIF metadata
- `src/pages/api/unlock.ts` - SSR password verification (sets HttpOnly cookie)
- `src/pages/api/check-password.ts` - Validate album passwords (legacy)
- `src/pages/api/download-album.ts` - Create ZIP of album photos

**Configuration:**
- `src/config.ts` - Centralized site configuration (URL, name, social defaults)
- `.env.example` - Environment variables template for deployment

**Utilities:**
- `src/lib/albums.ts` - Album/photo discovery, breadcrumbs, cover photos, password checking
- `src/lib/rate-limit.ts` - In-memory rate limiting (10 attempts / 15 minutes per IP)

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
- `scripts/update-albums.mjs` - Album structure normalization
- `scripts/fix-server-paths.mjs` - Production path fixer

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
