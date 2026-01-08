# Changelog

All notable changes to Goldplated Photos will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.7] - 2026-01-08

### Added

- **Parallel rsync deployment mode** (`--parallel` flag)
  - Splits albums by top-level directories (ws, neos, friends, etc.)
  - Runs up to 5 concurrent rsync workers
  - Each worker handles complete subtree with `--delete`
  - Final cleanup pass removes stale remote items
  - ~2x faster on large syncs
  - Usage: `npm run deploy:parallel` or `npm run deploy -- --parallel`

### Changed

- **Simplified deploy script** - replaced three-phase sync with cleaner approach
  - Sequential mode: Single rsync for entire albums tree (default)
  - Parallel mode: Top-level split with worker pool (opt-in)
  - Removed complex collection folder handling

---

## [0.4.6] - 2026-01-07

### Changed

- **Deploy script redesigned with three-phase sync** (industry best practice)
  - **Phase 1 (Parallel)**: Leaf albums sync with 4 workers, `--delete` flag
  - **Phase 2 (Sequential)**: Collection root files sync without `--delete`
  - **Phase 3 (Verification)**: Final rsync of entire albums tree with `--delete`
  - Based on research from [psuter.ch](https://wiki.psuter.ch/doku.php?id=parallel_rsync) parallel rsync

### Fixed

- **Critical: Deploy parallel sync race condition**
  - Only leaf albums (folders without subfolders) sync in parallel now
  - Collection folders tracked separately, synced sequentially
  - Final verification pass catches any missed files or orphans
- **Simplified collection sync** (removed complex `--filter` rules)
  - Old: `--delete --filter='P */' --filter='- */'` (error-prone)
  - New: `--exclude='*/'` only (simple, safe)
- **Comprehensive error handling across all sync phases**
  - Phase 1: Exit code checking (was already present)
  - Phase 2: Added error tracking for collection root syncs
  - Phase 3: Added error tracking for verification pass
  - `sync_simple`: Added error tracking for code/config syncs
  - All failures logged to error file with timestamps

---

## [0.4.5] - 2026-01-07

### Fixed

- Deploy script now uses `--delete` for collection folders (folders with subfolders)
  - Previously orphaned files in collection folders were never deleted
  - Now deploys properly clean up misplaced/old files

---

## [0.4.4] - 2026-01-07

### Added

- **Configurable rsync permissions** via `.env` file
  - `DEPLOY_CHMOD_DIRS` - Directory permissions (default: 775)
  - `DEPLOY_CHMOD_FILES` - File permissions (default: 664)
  - `DEPLOY_CHMOD_PRIVATE` - Private files like index.md, .htaccess (default: 660)
  - `DEPLOY_CHOWN` - Optional ownership setting (e.g., `user:www-data`)
- Deploy script now displays permission configuration at startup
- Support for different server setups (shared hosting vs Apache/Nginx www-data)

---

## [0.4.3] - 2026-01-06

### Added

- Deploy script now supports `--checksum` flag for thorough album sync
  - Usage: `npm run deploy -- --checksum`
  - Forces checksum comparison for album files (photos/videos)
  - Useful when files may be corrupted or need verification
  - Default behavior unchanged (checksum only for code files)

---

## [0.4.2] - 2026-01-06

### Fixed

- Deploy script now recursively expands album folders at any depth
  - Previously only handled 2 levels (e.g., `neos/razno/` synced as one task)
  - Now each leaf album syncs separately (e.g., `neos/razno/kraljevi-2026/`)
  - Prevents partial syncs when connection drops on large folders

---

## [0.4.1] - 2026-01-05

### Fixed

- Deploy script now uses checksum comparison for code files to prevent stale builds
  - Added `--checksum` flag to server, client, and scripts rsync operations
  - Album syncs unchanged (checksum would slow down large photo/video transfers)

---

## [0.4.0] - 2026-01-05

### Added

- **Mobile Hamburger Menu** - Marketing site now has mobile navigation menu
  - Slide-down menu with close-on-click behavior
  - Accessible with aria-expanded state
- **Swipeable Workflow Section** - Workflow steps are now swipeable cards on mobile
  - CSS scroll-snap for native swipe feel
  - Full descriptions visible on mobile
- **Hero Carousel Swipe** - Screenshot carousel now supports touch swipe on mobile
  - Pause auto-rotate during interaction
  - Left/right swipe navigation

### Fixed

- **Lightbox Share Button** - Share modal now displays correctly when triggered from lightbox
  - Fixed z-index to be higher than PhotoSwipe (100002 vs 100000)
- **Album/Folder Share** - Share buttons on album cards now use new share modal
  - Replaced old clipboard-only share with full modal
  - Copy Link, Facebook, LinkedIn options (no Instagram for albums)
  - Protection context passed for proper token handling

### Changed

- GitHub Discussions links replaced with Ko-fi donation links on marketing site
- Album share modal shows warning for protected content with token
- Nested albums inherit parent protection context for share links

---

## [0.3.0] - 2026-01-05

### Added

- **Enhanced Share Modal** - New share modal with multiple sharing options
  - Copy Link button with automatic token inclusion for protected albums
  - Facebook share integration
  - LinkedIn share integration
  - Instagram share with watermarked image download
- **Watermark API** - Server-side image watermarking for Instagram shares
  - Configurable watermark text (defaults to site URL)
  - Bottom-right positioning with 50% opacity
  - Clean sans-serif font with shadow for visibility
- **Protected Content Warning** - Alerts users when sharing password-protected content
- Share modal accessible from all locations: grid hover, lightbox toolbar, mobile long-press menu, and S keyboard shortcut

### Changed

- Share buttons now open modal instead of copying directly
- Protected album shares automatically include access token in URL

---

## [0.2.0] - 2026-01-05

### Added

- Support/donation links throughout documentation (Ko-fi, Wise)
- "Support the Project" section on docs homepage
- Ko-fi link in documentation footer
- Workflow section on docs homepage with 4-step process table

### Changed

- Improved marketing website with compact workflow infographic bar
- Enhanced features page with workflow descriptions
- Updated About page with donation options

---

## [0.1.0] - 2025-12-28

### Added

**Core Features**
- Photo grid with multiple view modes (grid, masonry, single-column, slideshow)
- PhotoSwipe lightbox integration with touch gestures
- Server-side password protection using SSR
- EXIF metadata display for photos
- Video playback support with metadata extraction
- Thumbnail generation system with three size presets (400px, 1200px, 1920px)
- Landing page with animated shutter button and sound effect
- Digital home with hero slider and content cards

**Gallery Experience**
- Keyboard navigation with spatial awareness
- Six sort options (name, date, file size - ascending/descending)
- View mode persistence via localStorage
- Mobile long-press context menus with haptic feedback
- Custom lightbox toolbar with image counter

**Lightbox Features**
- Original quality mode (`O` key) for full-resolution viewing
- Zen mode (`H` key) for distraction-free viewing
- Keyboard shortcuts overlay (`?` key)
- Layered escape key handling

**Security**
- Server-side access control for protected albums (SSR)
- Timing-safe password comparison
- Rate limiting on password attempts (10 attempts / 15 minutes)
- HttpOnly cookies for session management
- Path traversal protection on all API endpoints

**Content Management**
- Hierarchical album organization with collections
- Custom cover photo selection
- Album ordering and visibility controls
- Home page with hero slider and content cards
- Local admin panel (web-based CMS on port 4444)
- Album download as ZIP files

**Social Sharing**
- Open Graph meta tags for rich social previews
- Twitter Card meta tags
- Individual photo sharing via URL parameter

**Deployment**
- One-command production deployment (`npm run deploy`)
- rsync-based file synchronization
- PM2 process management integration
- Background server scripts

**Documentation**
- Comprehensive MkDocs documentation
- API endpoint reference
- Keyboard shortcuts reference
- Troubleshooting guide
- CLAUDE.md for AI-assisted development

### Security Notes

- Image URLs are NOT exposed in page source until authenticated
- Passwords stored in plaintext (intended for casual protection)
- Secure cookie flags: `httpOnly`, `secure` (prod), `sameSite: strict`

---

## Roadmap

Features planned for future releases:

- [ ] Search functionality across albums
- [ ] Bulk album operations
- [ ] Print-ready exports
- [ ] Multi-language support (i18n)
- [ ] Guest book / comments
- [ ] Photo watermarking
