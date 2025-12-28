# Welcome to Goldplated Photos

A beautiful, secure photo gallery for photographers and enthusiasts.

Built with [Astro](https://astro.build) + [PhotoSwipe](https://photoswipe.com)

---

## Features

### Gallery Experience

- **Multiple View Modes** - Grid, Masonry, and Single-column layouts
- **PhotoSwipe Lightbox** - Touch gestures, pinch-to-zoom, swipe navigation
- **EXIF Display** - Camera settings, lens info, and shooting parameters
- **Video Support** - Inline playback with metadata extraction
- **Keyboard Navigation** - Full accessibility with arrow keys, shortcuts

### Organization

- **Hierarchical Albums** - Nested folder structure for organization
- **Collections** - Group related albums together
- **Custom Ordering** - Control album display order
- **Cover Photos** - Auto-detected or manually selected

### Security

- **Password Protection** - Server-side access control (SSR)
- **Rate Limiting** - Protection against brute force attacks
- **Secure Cookies** - HttpOnly tokens prevent XSS
- **Path Traversal Protection** - All API endpoints secured

### Performance

- **Smart Thumbnails** - Three sizes (400px, 1200px, 1920px)
- **On-demand Generation** - Thumbnails created on first request
- **Persistent Caching** - Thumbnails cached for instant loading
- **EXIF Auto-rotation** - Correct orientation guaranteed

---

## Quick Start

```bash
# Clone the repository
git clone https://github.com/klukacin/goldplated-photos
cd goldplated-photos

# Install dependencies
npm install

# Start development server
npm run dev
```

Visit `http://localhost:4321` to see your gallery.

---

## Site Structure

| Route | Description |
|-------|-------------|
| `/` | Landing page with shutter button |
| `/home` | Digital home with hero slider and content cards |
| `/photos` | Photo gallery root |
| `/photos/*` | Individual albums and collections |

---

## Adding Photos

1. Create an album folder in `src/content/albums/`
2. Add an `index.md` file with metadata
3. Copy your photos to the folder
4. Photos are auto-discovered and thumbnails generated on first view

```yaml
# src/content/albums/2025/vacation/index.md
---
title: "Summer Vacation"
thumbnail: "beach-sunset.jpg"
---
```

---

## Documentation

- **[Installation](getting-started/installation.md)** - Full setup guide
- **[Quick Start](getting-started/quick-start.md)** - Get running in minutes
- **[Configuration](getting-started/configuration.md)** - All options explained
- **[Architecture](reference/architecture.md)** - Technical deep-dive

---

## License

MIT License - See [LICENSE](https://github.com/klukacin/goldplated-photos/blob/main/LICENSE)

---

*Made with love by Kristijan Lukacin & Claude Code*
