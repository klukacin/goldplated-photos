# Welcome to Goldplated Photos

A beautiful, secure, self-hosted photo gallery for photographers and enthusiasts.

**Built with [Astro](https://astro.build) + [PhotoSwipe](https://photoswipe.com) by [Kristijan Lukacin](https://kristijan.lukacin.com) and [Claude AI](https://claude.ai)**

[:material-web: Live Demo](https://kristijan.lukacin.com){ .md-button .md-button--primary }
[:material-github: GitHub](https://github.com/klukacin/goldplated-photos){ .md-button }

---

## Why Goldplated Photos?

<div class="grid cards" markdown>

-   :material-shield-lock:{ .lg .middle } **True Privacy**

    ---

    Self-hosted on your own server. No cloud services, no third-party tracking. Your photos stay yours.

-   :material-database-off:{ .lg .middle } **No Database**

    ---

    File-based architecture using folders and markdown. Easy to backup, version control, and migrate.

-   :material-rocket-launch:{ .lg .middle } **Modern Stack**

    ---

    Built with Astro 5, PhotoSwipe 5, and Sharp. Fast, modern, and actively maintained.

-   :material-robot:{ .lg .middle } **Human + AI Built**

    ---

    Crafted by Kristijan Lukacin with Claude AI assistance. Includes CLAUDE.md for AI-assisted development.

</div>

---

## Features

### Gallery Experience

- **Multiple View Modes** - Grid, Masonry, Single-column, and Slideshow layouts
- **PhotoSwipe Lightbox** - Touch gestures, pinch-to-zoom, swipe navigation
- **EXIF Display** - Camera settings, lens info, and shooting parameters
- **Video Support** - Inline playback with metadata extraction (ffprobe)
- **Keyboard Navigation** - Full accessibility with arrow keys and shortcuts

### Organization

- **Hierarchical Albums** - Nested folder structure for organization
- **Collections** - Group related albums together
- **Custom Ordering** - Control album display order via frontmatter
- **Cover Photos** - Auto-detected or manually selected

### Security

- **Password Protection** - Server-side access control (SSR)
- **Rate Limiting** - 10 failed attempts per 15 minutes per IP
- **Secure Cookies** - HttpOnly, Secure, SameSite strict flags
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

For detailed setup instructions, see the [Installation Guide](getting-started/installation.md).

---

## Site Structure

| Route | Description |
|-------|-------------|
| `/` | Landing page with animated shutter button |
| `/home` | Digital home with hero slider and content cards |
| `/photos` | Photo gallery root with album browser |
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
password: "secret"  # Optional
---
```

For more details, see [Adding Albums](guides/adding-albums.md).

---

## Admin Panel

A local web-based CMS for content management (never deployed to production).

```bash
npm run admin     # Start on port 4444
```

Features:

- Album creation and editing
- Photo/video upload via drag-and-drop
- Hero slider image management
- Content card editor
- Thumbnail cache management

See [Admin Panel Guide](guides/admin-panel.md) for full documentation.

---

## Documentation

### Getting Started

- [Installation](getting-started/installation.md) - Full setup guide with prerequisites
- [Quick Start](getting-started/quick-start.md) - Get running in 5 minutes
- [Configuration](getting-started/configuration.md) - Environment variables and options

### Guides

- [Adding Albums](guides/adding-albums.md) - Album structure and frontmatter
- [Admin Panel](guides/admin-panel.md) - Using the local CMS
- [Deployment](guides/deployment.md) - Production server setup
- [Customization](guides/customization.md) - Theming and styling
- [Using Claude AI](guides/using-claude.md) - AI-assisted development

### Reference

- [Architecture](reference/architecture.md) - Technical deep-dive
- [API Endpoints](reference/api-endpoints.md) - All API routes
- [Keyboard Shortcuts](reference/keyboard-shortcuts.md) - Navigation keys
- [UI Components](reference/ui-components.md) - Component documentation

---

## Contributing

We welcome contributions! See the [Contributing Guide](https://github.com/klukacin/goldplated-photos/blob/main/CONTRIBUTING.md) for guidelines.

This project includes [CLAUDE.md](https://github.com/klukacin/goldplated-photos/blob/main/CLAUDE.md) for AI-assisted development with Claude Code.

---

## License

MIT License - See [LICENSE](https://github.com/klukacin/goldplated-photos/blob/main/LICENSE)

---

*Created by [Kristijan Lukacin](https://kristijan.lukacin.com) with [Claude AI](https://claude.ai) assistance*
