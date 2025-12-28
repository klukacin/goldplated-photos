# Changelog

All notable changes to Goldplated Photos will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] - 2025-12-28

### Added

**Core Features**
- Photo grid with multiple view modes (grid, masonry, single-column)
- PhotoSwipe lightbox integration with touch gestures
- Server-side password protection using SSR
- EXIF metadata display for photos
- Video playback support with metadata extraction
- Thumbnail generation system with three size presets

**Gallery Experience**
- Keyboard navigation with spatial awareness
- Sort options (name, date, file size)
- View mode persistence via localStorage
- Mobile long-press context menus
- Haptic feedback on mobile interactions

**Lightbox Features**
- Original quality mode for full-resolution viewing
- Zen mode for distraction-free viewing
- Custom toolbar with image counter
- Keyboard shortcuts overlay

**Security**
- Server-side access control for protected albums
- Timing-safe password comparison
- Rate limiting on password attempts (10/15min)
- HttpOnly cookies for session management
- Path traversal protection on all API endpoints

**Content Management**
- Hierarchical album organization
- Collections for grouping albums
- Custom cover photo selection
- Album ordering and visibility controls
- Home page with hero slider and content cards

**Developer Experience**
- Astro-based architecture with hybrid rendering
- Content Collections for albums and home content
- Comprehensive API endpoints
- Sharp-based image processing
- MkDocs documentation

### Security

- Image URLs not exposed in page source until authenticated
- Rate limiting prevents brute force attacks
- Secure cookie flags (httpOnly, secure, sameSite)
- Input validation on all API endpoints

---

## Upcoming

Features planned for future releases:

- [ ] Image upload interface
- [ ] Bulk album operations
- [ ] Search functionality
- [ ] Social sharing integration
- [ ] Print-ready exports
- [ ] Multi-language support

---

## Contributing

See [GitHub repository](https://github.com/klukacin/goldplated-photos) for contribution guidelines.
