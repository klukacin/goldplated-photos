# Changelog

All notable changes to Goldplated Photos will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-12-28

### Added

**Core Features**
- Photo grid with multiple view modes (grid, masonry, single-column)
- PhotoSwipe lightbox integration with touch gestures
- Server-side password protection using SSR
- EXIF metadata display for photos
- Video playback support with metadata extraction
- Thumbnail generation system with three size presets (400px, 1200px, 1920px)

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

**Documentation**
- Comprehensive MkDocs documentation
- API endpoint reference
- Keyboard shortcuts reference
- Troubleshooting guide

### Security Notes

- Image URLs are NOT exposed in page source until authenticated
- Passwords stored in plaintext (intended for casual protection)
- Secure cookie flags: `httpOnly`, `secure` (prod), `sameSite: strict`

---

## Roadmap

Features planned for future releases:

- [ ] Image upload interface
- [ ] Bulk album operations
- [ ] Search functionality
- [ ] Social sharing integration
- [ ] Print-ready exports
- [ ] Multi-language support
