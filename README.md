# Goldplated Photos

A beautiful, secure photo gallery for photographers and enthusiasts.

Built with [Astro](https://astro.build) + [PhotoSwipe](https://photoswipe.com)

## Features

**Gallery Experience**
- Multiple view modes (Grid, Masonry, Single-column)
- PhotoSwipe lightbox with touch gestures and pinch-to-zoom
- Keyboard navigation with spatial awareness
- EXIF metadata display (camera, lens, settings)
- Video playback support

**Security**
- Server-side password protection (SSR)
- Rate limiting on password attempts
- Secure HttpOnly cookies
- Path traversal protection

**Performance**
- Smart thumbnail generation (400px, 1200px, 1920px)
- On-demand processing with persistent caching
- Automatic EXIF orientation correction

**Organization**
- Hierarchical album structure
- Collections for grouping albums
- Custom cover photos
- Album ordering and visibility controls

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

## Adding Albums

Create a folder in `src/content/albums/` with an `index.md` file:

```
src/content/albums/
└── 2025/
    └── vacation/
        ├── index.md      # Album metadata
        ├── photo1.jpg
        └── photo2.jpg
```

```yaml
# index.md
---
title: "Summer Vacation"
thumbnail: "beach-sunset.jpg"  # Optional cover photo
password: "secret"             # Optional password
---
```

## Commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start development server |
| `npm run build` | Build for production |
| `npm run preview` | Preview production build |

## Documentation

Full documentation available at [klukacin.github.io/goldplated-photos](https://klukacin.github.io/goldplated-photos)

- [Installation Guide](docs/docs/getting-started/installation.md)
- [Configuration](docs/docs/getting-started/configuration.md)
- [Adding Albums](docs/docs/guides/adding-albums.md)
- [API Reference](docs/docs/reference/api-endpoints.md)

## Tech Stack

- **Astro** - Static site generator with SSR support
- **PhotoSwipe** - Touch-friendly lightbox
- **Sharp** - High-performance image processing
- **Node.js** - Server runtime

## License

MIT License - see [LICENSE](LICENSE) for details.

## Author

**Kristijan Lukacin** - [GitHub](https://github.com/klukacin)

---

Made with Claude Code
