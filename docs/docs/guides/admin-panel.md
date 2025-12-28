# Admin Panel

A web-based content management system for managing your gallery locally.

---

## Overview

The Admin Panel is a **local-only** tool that provides a browser-based interface for managing:

- Albums and collections
- Photos and videos
- Home page content (hero slider, intro text, content cards)
- Thumbnail cache

!!! warning "Local Development Only"
    The admin panel is **never deployed** to production. It runs locally on your machine and edits files directly in `src/content/`. Changes are synced to production via `npm run deploy`.

---

## Starting the Admin Panel

```bash
# Run in foreground (see logs in terminal)
npm run admin

# Run in background
npm run admin:bg

# Stop background server
npm run stop:admin
```

**Access:** `http://localhost:4444`

**Prerequisites:** The dev server should be running (`npm run dev`) to preview changes.

---

## Interface Overview

The admin panel has three main tabs:

| Tab | Purpose |
|-----|---------|
| **Albums** | Manage photo albums and collections |
| **Home** | Edit landing page, hero slider, intro, and content cards |
| **Tools** | Cache management and quick links |

---

## Albums Tab

### Album Tree (Left Sidebar)

Browse your album hierarchy:

- Organized by year/category/album
- Shows photo/video counts
- Click to select and edit
- **+ New Album** button to create albums

### Album Editor (Right Panel)

#### Settings Sub-tab

| Field | Description |
|-------|-------------|
| **Title** | Album display name |
| **Date** | Album date |
| **Description** | Brief description |
| **Password** | Optional password protection |
| **Token** | Auto-generated shareable token |
| **Order** | Display order (lower = first) |
| **Sort By** | Photo sort order (date, name, custom) |
| **Style** | Display style (grid, masonry, slideshow, single-column) |
| **Cover Photo** | Select from album photos or auto-detect |
| **Tags** | Comma-separated tags |

**Checkboxes:**

- **Is Collection** - Contains sub-albums instead of photos
- **Allow Anonymous** - Token bypasses parent password
- **Hidden** - Not listed, accessible via direct link only

**Body Content:** Markdown editor for album description.

#### Photos Sub-tab

- View all photos in the album
- **Drag-and-drop upload** (up to 50 files)
- Delete individual photos
- Shows file size and date

#### Videos Sub-tab

- View all videos with inline preview player
- Drag-and-drop video upload
- Delete videos
- Shows file size and date

#### Cache Sub-tab

- View thumbnail cache stats for this album
- Shows count of small (400px), medium (1200px), large (1920px)
- **Clear Cache** button to regenerate thumbnails

---

## Home Tab

Manage all content on the `/home` page.

### Landing Background

Upload or replace the full-screen background image for the landing page (`/`).

**Location:** `public/images/landing-bg.jpg`

### Hero Slider

Manage images for the home page carousel.

- Upload new images
- Delete existing images
- Images stored in `public/home/hero/`

### Introduction

Edit the intro paragraph displayed on the home page.

- Markdown editor with preview
- Stored in `src/content/home/intro.md`

### Content Cards

Manage the alternating image/text cards on the home page.

**Card Properties:**

| Field | Description |
|-------|-------------|
| Title | Card heading |
| Image | Select from uploaded card images |
| Image Position | Left or right |
| Link | Destination URL (e.g., `/photos/weddings`) |
| Order | Display order |
| Body | Markdown content |

**Actions:**

- **Add Card** - Create new card
- **Edit** - Open card editor modal
- **Delete** - Remove card
- **Drag to reorder** - Change display order

**Card Images:** Uploaded to `public/home/cards/`

---

## Tools Tab

### Thumbnail Cache

View and manage the global thumbnail cache.

**Statistics:**

- Total cached thumbnails
- Count by size (small, medium, large)

**Actions:**

- **Refresh Stats** - Update counts
- **Clear All** - Delete all cached thumbnails (regenerated on next request)

### Quick Links

Direct links to:

- Dev site (`localhost:4321`)
- Photo gallery (`/photos`)
- Landing page (`/`)
- Home page (`/home`)

---

## Recommended Workflow

```bash
# 1. Start both servers
npm run dev:bg
npm run admin:bg

# 2. Open in browser
# Admin: http://localhost:4444
# Preview: http://localhost:4321

# 3. Make changes in admin panel
# - Edit albums, upload photos
# - Update home page content

# 4. Preview changes (auto-refresh)

# 5. Deploy to production
npm run deploy

# 6. Stop servers when done
npm run stop:dev
npm run stop:admin
```

---

## Admin API Endpoints

The admin panel uses these internal API endpoints:

### Albums

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/albums` | GET | List all albums |
| `/api/albums/*path` | GET | Get album metadata |
| `/api/albums` | POST | Create album |
| `/api/albums/*path` | PUT | Update album |
| `/api/albums/*path` | DELETE | Delete album |

### Photos & Videos

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/photos/*path` | GET | List photos |
| `/api/photos/*path` | POST | Upload photos |
| `/api/photos/:path/file/:name` | DELETE | Delete photo |
| `/api/videos/*path` | GET | List videos |

### Home Content

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/home/intro` | GET/PUT | Intro text |
| `/api/home/cards` | GET/POST | List/create cards |
| `/api/home/cards/:id` | PUT/DELETE | Update/delete card |
| `/api/home/cards/reorder` | POST | Reorder cards |

### Assets

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/assets/hero` | GET/POST | Hero images |
| `/api/assets/hero/:name` | DELETE | Delete hero image |
| `/api/assets/cards` | GET/POST | Card images |
| `/api/assets/landing` | POST | Landing background |

### Cache

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/cache/stats` | GET | Global cache stats |
| `/api/cache/thumbnails` | DELETE | Clear all cache |
| `/api/cache/album/*path` | GET/DELETE | Album-specific cache |

---

## Technical Details

**Server:** Express.js on port 4444

**Frontend:** Vanilla JavaScript with CodeMirror for markdown editing

**Storage:** File-based (no database) - edits go directly to:
- `src/content/albums/` - Album metadata and photos
- `src/content/home/` - Home page content
- `public/` - Static assets

---

## Troubleshooting

### Admin Won't Start

1. Check port 4444 is available: `lsof -i :4444`
2. Kill existing process if needed
3. Try running in foreground to see errors: `npm run admin`

### Changes Not Showing

1. Ensure dev server is running (`npm run dev`)
2. Hard refresh browser (Cmd+Shift+R)
3. Check terminal for errors

### Upload Fails

1. Check file size (large files may timeout)
2. Verify file format is supported
3. Check `src/content/albums/` permissions

---

## Related

- [Deployment](deployment.md) - Deploy changes to production
- [Adding Albums](adding-albums.md) - Manual album creation
- [Architecture](../reference/architecture.md) - Technical details
