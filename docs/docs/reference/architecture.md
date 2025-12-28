# Architecture

Technical deep-dive into Goldplated Photos.

---

## Overview

Goldplated Photos is built with:

- **[Astro](https://astro.build)** - Static site generator with SSR support
- **[PhotoSwipe](https://photoswipe.com)** - Lightbox component
- **[Sharp](https://sharp.pixelplumbing.com/)** - Image processing
- **Node.js** - Server runtime

---

## Rendering Modes

### Hybrid Rendering

The site uses Astro's hybrid rendering:

- **Static pages** - Pre-rendered at build time (`prerender = true`)
- **SSR pages** - Rendered on each request (`prerender = false`)

### SSR for Security

Protected albums use SSR to prevent content exposure:

```typescript
// src/pages/photos/[...path].astro
export const prerender = false; // SSR mode
```

Content is only rendered after access verification.

---

## Content Collections

### Albums Collection

```
src/content/albums/
└── {year}/
    └── {category}/
        └── {album-name}/
            ├── index.md          # Metadata (required)
            └── *.jpg             # Photos
```

### Schema Definition

```typescript
// src/content/config.ts
const albumsCollection = defineCollection({
  type: 'content',
  schema: z.object({
    title: z.string(),
    password: z.string().optional(),
    thumbnail: z.string().optional(),
    order: z.number().optional(),
    hidden: z.boolean().default(false),
    isCollection: z.boolean().default(false),
    allowAnonymous: z.boolean().default(false),
    style: z.enum(['grid', 'masonry', 'single']).default('grid'),
    token: z.string().default(() => crypto.randomUUID())
  })
});
```

### Home Collection

```
src/content/home/
├── intro.md              # Introduction text
└── cards/
    └── *.md              # Content cards
```

---

## Routing

| Route | File | Mode |
|-------|------|------|
| `/` | `src/pages/index.astro` | Static |
| `/home` | `src/pages/home.astro` | Static |
| `/photos` | `src/pages/photos/index.astro` | Static |
| `/photos/*` | `src/pages/photos/[...path].astro` | SSR |
| `/albums/*` | `src/pages/albums/[...path].ts` | SSR |
| `/api/*` | `src/pages/api/*.ts` | SSR |

---

## API Endpoints

### Thumbnail Generation

**Endpoint:** `GET /api/thumbnail`

```typescript
// Query params
?path=2025/album/photo.jpg&size=small
```

Flow:

1. Check cache (`.meta/thumbnails/{size}/{hash}.jpg`)
2. If cached, serve file
3. If not, generate with Sharp, cache, serve

### EXIF Extraction

**Endpoint:** `GET /api/exif`

```typescript
// Query params
?path=2025/album/photo.jpg
```

Uses `exifr` library to extract metadata.

### Password Unlock

**Endpoint:** `POST /api/unlock`

```typescript
// Form data
albumPath: string
password: string
returnUrl: string
```

Flow:

1. Rate limit check
2. Fetch album metadata
3. Timing-safe password comparison
4. Set HttpOnly cookie with token
5. Redirect to album

### Album Download

**Endpoint:** `GET /api/download-album`

Requires `X-Album-Token` header for protected albums.

---

## Security Features

### Rate Limiting

```typescript
// src/lib/rate-limit.ts
const MAX_ATTEMPTS = 10;
const LOCKOUT_DURATION = 15 * 60 * 1000; // 15 minutes
```

In-memory Map tracks attempts per IP.

### Timing-Safe Comparison

```typescript
import { timingSafeEqual } from 'crypto';

function safeCompare(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  return timingSafeEqual(Buffer.from(a), Buffer.from(b));
}
```

### Path Traversal Protection

All file access APIs validate paths:

```typescript
if (path.includes('..') || path.startsWith('/')) {
  return new Response('Invalid path', { status: 400 });
}
```

### Cookie Security

```typescript
cookies.set('album-access', JSON.stringify(tokens), {
  httpOnly: true,          // No JS access
  secure: import.meta.env.PROD,  // HTTPS in prod
  sameSite: 'strict',      // CSRF protection
  maxAge: 86400,           // 24 hours
  path: '/'
});
```

---

## Key Components

### PhotoGrid.astro

Main gallery component handling:

- Photo display in grid/masonry/single layouts
- PhotoSwipe lightbox initialization
- Keyboard navigation
- EXIF display
- Video playback

### AlbumGrid.astro

Displays sub-albums with:

- Cover photo thumbnails
- Album titles
- Link to album pages

### Breadcrumbs.astro

Hierarchical navigation showing path from root.

---

## Utility Functions

### src/lib/albums.ts

```typescript
// Get album by path
getAlbumByPath(path: string): Promise<Album>

// Get photos in album
getPhotosForAlbum(albumPath: string): Promise<Photo[]>

// Get ancestor albums
getAncestors(path: string): Promise<Album[]>

// Get child albums
getAllDescendants(path: string): Promise<Album[]>

// Generate breadcrumb data
getBreadcrumbs(path: string): Breadcrumb[]
```

### src/lib/rate-limit.ts

```typescript
// Check if IP is rate limited
isRateLimited(ip: string): boolean

// Record failed attempt
recordFailedAttempt(ip: string): void

// Clear rate limit (on success)
clearRateLimit(ip: string): void

// Get remaining attempts
getRemainingAttempts(ip: string): number
```

---

## Image Processing Pipeline

### Original → Thumbnail

```
Original Image
     ↓
Sharp reads file
     ↓
.rotate() - Auto-orient from EXIF
     ↓
.resize(maxWidth) - Scale down
     ↓
.jpeg({ quality: 85 }) - Compress
     ↓
Write to .meta/thumbnails/
     ↓
Serve to client
```

### Thumbnail Sizes

| Size | Max Width | Use |
|------|-----------|-----|
| small | 400px | Grid |
| medium | 1200px | Preview |
| large | 1920px | Lightbox |

---

## Event Handling

### Keyboard Events

PhotoGrid uses event capture phase to intercept before PhotoSwipe:

```javascript
window.addEventListener('keydown', handleKeyDown, true); // capture
```

### Escape Key Layering

1. First: Close help overlay
2. Second: Close EXIF overlay
3. Third: Close lightbox

---

## Data Flow

### Album Page Load

```
Request → SSR Handler
              ↓
    Check album-access cookie
              ↓
    Access granted? ─── No ──→ Render password form
              ↓
             Yes
              ↓
    Fetch album metadata
              ↓
    Fetch photos list
              ↓
    Render full page with images
```

### Password Submission

```
Form POST → /api/unlock
              ↓
    Rate limit check
              ↓
    Fetch album
              ↓
    Compare password (timing-safe)
              ↓
    Success? ─── No ──→ Redirect with error
              ↓
             Yes
              ↓
    Set cookie with token
              ↓
    Redirect to album
```

---

## File Structure

```
src/
├── components/
│   ├── PhotoGrid.astro      # Photo display
│   ├── AlbumGrid.astro      # Album grid
│   ├── Breadcrumbs.astro    # Navigation
│   └── Footer.astro         # Site footer
├── content/
│   ├── albums/              # Photo albums
│   └── home/                # Home page content
├── layouts/
│   └── Layout.astro         # Base layout
├── lib/
│   ├── albums.ts            # Album utilities
│   └── rate-limit.ts        # Rate limiting
└── pages/
    ├── index.astro          # Landing page
    ├── home.astro           # Home page
    ├── photos/              # Gallery pages
    ├── albums/              # Image serving
    └── api/                 # API endpoints
```

---

## Admin Panel

A separate Express.js server for local content management.

### Overview

| Component | Description |
|-----------|-------------|
| **Server** | Express.js on port 4444 |
| **Frontend** | Vanilla JS + CodeMirror |
| **Storage** | File-based (no database) |
| **Deployment** | Local only (never deployed) |

### Architecture

```
admin/
├── server.js           # Express server
├── index.html          # Single-page admin UI
├── admin.css           # Styles
└── js/
    ├── app.js          # Main controller
    ├── albums.js       # Album management
    ├── photos.js       # Photo upload/delete
    ├── home.js         # Home page content
    └── utils.js        # API client, utilities
```

### Admin API Endpoints

The admin server provides its own API:

**Albums:**
- `GET/POST /api/albums` - List/create albums
- `GET/PUT/DELETE /api/albums/*path` - Album CRUD

**Content:**
- `GET/POST /api/photos/*path` - Photos
- `GET/POST /api/videos/*path` - Videos
- `GET/PUT /api/home/intro` - Intro text
- `GET/POST/PUT/DELETE /api/home/cards` - Content cards

**Assets:**
- `GET/POST/DELETE /api/assets/hero` - Hero images
- `GET/POST/DELETE /api/assets/cards` - Card images
- `POST /api/assets/landing` - Landing background

**Cache:**
- `GET/DELETE /api/cache/stats` - Global cache
- `GET/DELETE /api/cache/album/*path` - Per-album cache

### Data Flow

```
Admin Panel (browser)
       ↓
Admin API (Express :4444)
       ↓
File System
├── src/content/albums/    # Album metadata & photos
├── src/content/home/      # Home page content
└── public/                # Static assets
       ↓
Dev Server (Astro :4321)   # Auto-reloads on changes
```

!!! note "Local Only"
    The admin panel is never deployed. All changes are synced to production via `npm run deploy`.

---

## Related

- [Admin Panel Guide](../guides/admin-panel.md) - User guide
- [API Endpoints](api-endpoints.md) - Full API reference
- [Configuration](../getting-started/configuration.md) - Config options
