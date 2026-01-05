# Goldplated Photos

A beautiful, secure, self-hosted photo gallery for photographers and enthusiasts.

**Built with [Astro](https://astro.build) + [PhotoSwipe](https://photoswipe.com) by [Kristijan Lukacin](https://kristijan.lukacin.com) and [Claude AI](https://claude.ai)**

[Live Demo](https://kristijan.lukacin.com) | [Documentation](https://docs.goldplated.photos) | [Website](https://goldplated.photos)

---

## Why Goldplated Photos?

| | |
|---|---|
| **True Privacy** | Self-hosted. No cloud services. Your photos stay on your server. |
| **No Database** | File-based architecture. Just folders and markdown files. |
| **Modern Stack** | Built with Astro 5, PhotoSwipe 5, and Sharp for fast image processing. |
| **Production Ready** | Rate limiting, security headers, EXIF auto-rotation out of the box. |
| **One-Command Deploy** | rsync-based deployment with PM2 process management. |
| **Human + AI Built** | Crafted with Claude AI assistance, documented for AI-assisted development. |

---

## Features

### Gallery Experience
- **Multiple View Modes** - Grid, Masonry, Single-column, and Slideshow
- **PhotoSwipe Lightbox** - Touch gestures, pinch-to-zoom, swipe navigation
- **EXIF Display** - Camera settings, lens info, shooting parameters
- **Video Support** - Inline playback with metadata extraction (ffprobe)
- **Keyboard Navigation** - Full accessibility with arrow keys and shortcuts

### Security
- **SSR Password Protection** - Image URLs never exposed until authenticated
- **Rate Limiting** - 10 failed attempts per 15 minutes per IP
- **Secure Cookies** - HttpOnly, Secure, SameSite strict flags
- **Path Traversal Protection** - All API endpoints secured

### Performance
- **Smart Thumbnails** - Three sizes (400px, 1200px, 1920px)
- **Persistent Caching** - Thumbnails cached on first request
- **EXIF Auto-rotation** - Correct orientation guaranteed
- **Lazy Loading** - Images load as they enter viewport

### Organization
- **Hierarchical Albums** - Nested folder structure
- **Collections** - Group related albums together
- **Custom Cover Photos** - Auto-detected or manually selected
- **Album Ordering** - Control display order with frontmatter

---

## Quick Start

### Prerequisites

- Node.js 18+ (LTS recommended)
- npm 9+
- ffmpeg (optional, for video metadata)

### Installation

```bash
# Clone the repository
git clone https://github.com/klukacin/goldplated-photos.git
cd goldplated-photos

# Install dependencies
npm install

# Start development server
npm run dev
```

Visit `http://localhost:4321` to see your gallery.

### Adding Photos

1. Create an album folder in `src/content/albums/`
2. Add an `index.md` file with metadata
3. Copy your photos to the folder

```yaml
# src/content/albums/2025/vacation/index.md
---
title: "Summer Vacation"
thumbnail: "beach-sunset.jpg"
password: "secret"  # Optional
---
```

Photos are auto-discovered and thumbnails generated on first view.

---

## Admin Panel

A local web-based CMS for content management (never deployed to production).

```bash
npm run admin     # Start on port 4444
```

**Features:**
- Album creation and editing
- Photo/video upload
- Hero slider management
- Content card editor
- Thumbnail cache management

---

## Commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start development server (port 4321) |
| `npm run dev:bg` | Start dev server in background |
| `npm run admin` | Start admin panel (port 4444) |
| `npm run build` | Build for production |
| `npm run deploy` | Deploy to production server |

---

## Deployment

### Production Server Requirements

- Node.js 18+ runtime
- PM2 process manager
- SSH access with rsync

### Deploy

```bash
# Configure .env with server details
cp .env.example .env

# Deploy to production
npm run deploy
```

See [Deployment Guide](https://docs.goldplated.photos/guides/deployment) for full instructions.

---

## Documentation

Full documentation available at [docs.goldplated.photos](https://docs.goldplated.photos)

- [Installation Guide](https://docs.goldplated.photos/getting-started/installation)
- [Configuration](https://docs.goldplated.photos/getting-started/configuration)
- [Adding Albums](https://docs.goldplated.photos/guides/adding-albums)
- [Admin Panel](https://docs.goldplated.photos/guides/admin-panel)
- [Deployment](https://docs.goldplated.photos/guides/deployment)
- [API Reference](https://docs.goldplated.photos/reference/api-endpoints)
- [Keyboard Shortcuts](https://docs.goldplated.photos/reference/keyboard-shortcuts)

---

## Tech Stack

| Technology | Purpose |
|------------|---------|
| [Astro](https://astro.build) | Static site generator with SSR |
| [PhotoSwipe](https://photoswipe.com) | Touch-friendly lightbox |
| [Sharp](https://sharp.pixelplumbing.com) | Image processing |
| [exifr](https://github.com/MikeKovaworker/exifr) | EXIF extraction |
| [ffprobe](https://ffmpeg.org/ffprobe.html) | Video metadata |
| [Express](https://expressjs.com) | Admin panel server |

---

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Working with Claude AI

This project includes comprehensive documentation for AI-assisted development. See [CLAUDE.md](CLAUDE.md) for project context that Claude Code uses automatically.

```bash
# Start Claude Code session
claude

# Claude will read CLAUDE.md and understand the project
```

---

## Credits

**Created by [Kristijan Lukacin](https://kristijan.lukacin.com)** with [Claude AI](https://claude.ai) assistance.

This project demonstrates human-AI collaboration in software development. Both the code and documentation were crafted through iterative conversations with Claude, Anthropic's AI assistant.

---

## Support

If you find Goldplated Photos useful, consider supporting its development:

- [Buy me a coffee on Ko-fi](https://ko-fi.com/klukacin)
- [Send a tip via Wise](https://wise.com/pay/me/kristijanl10)

Your support helps maintain and improve this project!

---

## License

MIT License - see [LICENSE](LICENSE) for details.

---

**[goldplated.photos](https://goldplated.photos)** | **[docs.goldplated.photos](https://docs.goldplated.photos)** | **[Live Demo](https://kristijan.lukacin.com)**
