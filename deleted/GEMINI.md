# Project Context: Astro Photo Gallery

## Project Overview
This is a high-performance, static-site generated (SSG) photo gallery built with **Astro 5** and **TypeScript**. It features a nested album structure, EXIF data extraction, password protection, and an efficient on-demand thumbnail generation system.

### Tech Stack
- **Framework:** Astro 5 (`@astrojs/node` adapter)
- **Language:** TypeScript
- **Image Processing:** `sharp` (via `src/pages/api/thumbnail.ts`) & `exifr` (metadata)
- **Viewer:** PhotoSwipe
- **Deployment Target:** Node.js (Standalone) or Static Hosting (with Server Functions)

## Key Directories & Files

| Path | Description |
| :--- | :--- |
| `src/content/albums/` | **Core Content Source.** Nested folders containing photos and `index.md` metadata files. |
| `src/lib/albums.ts` | **Data Logic.** Functions to fetch albums, photos, handle sorting, and check access permissions. |
| `src/content/config.ts` | **Schema Definition.** Zod schemas for album metadata (frontmatter). |
| `src/pages/api/thumbnail.ts` | **Thumbnail Service.** API endpoint for generating/serving resized images. |
| `admin/` | **Admin Dashboard.** Express.js server (`server.js`) and frontend for managing albums via UI. |
| `astro.config.mjs` | Astro configuration. Set to `output: 'server'` with `standalone` node adapter. |

## System Architectures

### 1. Content System (Albums & Photos)
- **Structure:** Hierarchical directory structure in `src/content/albums/`.
- **Metadata:** Each folder MUST have an `index.md` with frontmatter defined in `src/content/config.ts`.
- **Collections:** Folders can be simple albums (contain photos) or "Collections" (contain other albums, `isCollection: true`).
- **Discovery:** `src/lib/albums.ts` scans the filesystem to build the gallery hierarchy.

### 2. Thumbnail System
- **Mechanism:** On-demand generation via `/api/thumbnail?path=...&size=...`.
- **Sizes:** `small` (grid), `medium` (lightbox), `large` (fullscreen).
- **Caching:** Generated thumbnails are stored in `.meta/thumbnails/` **inside each album folder**.
- **Performance:** Reduces bandwidth by ~99.9% for grid views.
- **Maintenance:** Delete `.meta/` folders to force regeneration.

### 3. Access Control
- **Tokens:** Each album has a unique `token` in frontmatter.
- **Passwords:** Defined in `index.md`.
- **Inheritance:** Passwords flow down from parent albums.
- **Bypass:** `allowAnonymous: true` allows direct access via token, bypassing parent passwords.

## Development Workflow

### Standard Commands
```bash
npm install       # Install dependencies
npm run dev       # Start Astro dev server (localhost:4321)
npm run build     # Build for production (output to dist/)
npm run preview   # Preview the production build
```

### Admin Interface
A custom admin interface exists to manage content without editing Markdown files manually.
```bash
npm run admin     # Starts the admin server (likely on a different port, check console)
```

## Critical Conventions
1.  **Image Files:** Place directly in album folders. Supported: `.jpg`, `.png`, `.webp`, etc.
2.  **Metadata:** ALWAYS keep `index.md` valid. It drives the entire gallery logic.
3.  **Git:** The `.meta/` directories in albums are git-ignored. Do not commit thumbnails.
4.  **Styles:** Layouts are in `src/layouts/Layout.astro`. CSS variables control the theme.
