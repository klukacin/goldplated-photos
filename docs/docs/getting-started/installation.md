# Installation

Complete guide to setting up Goldplated Photos on your system.

---

## Requirements

- **Node.js** 18+ (LTS recommended)
- **npm** 9+ or **pnpm**
- **ffprobe** (optional, for video metadata)

---

## Quick Install

```bash
# Clone the repository
git clone https://github.com/klukacin/goldplated-photos
cd goldplated-photos

# Install dependencies
npm install

# Start development server
npm run dev
```

The gallery will be available at `http://localhost:4321`

---

## Project Structure

```
goldplated-photos/
├── src/
│   ├── content/
│   │   ├── albums/          # Photo albums go here
│   │   │   └── {year}/
│   │   │       └── {category}/
│   │   │           └── {album-name}/
│   │   │               ├── index.md    # Album metadata
│   │   │               └── *.jpg       # Photos
│   │   └── home/            # Home page content
│   ├── components/          # Astro components
│   ├── layouts/             # Page layouts
│   ├── lib/                 # Utility functions
│   └── pages/               # Routes
├── public/                  # Static assets
│   ├── images/              # Landing page images
│   ├── sounds/              # Audio files
│   └── home/                # Home page assets
├── .meta/                   # Generated thumbnails (gitignored)
└── docs/                    # Documentation
```

---

## All npm Commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start Astro dev server (foreground, port 4321) |
| `npm run dev:bg` | Start dev server in background |
| `npm run stop:dev` | Stop background dev server |
| `npm run build` | Build production site to `./dist/` |
| `npm run preview` | Preview production build locally |
| `npm run deploy` | Full deployment to production server |
| `npm run update` | Normalize album structure (auto-create index.md) |
| `npm run admin` | Start admin panel (foreground, port 4444) |
| `npm run admin:bg` | Start admin panel in background |
| `npm run stop:admin` | Stop background admin panel |

---

## Development Setup

The recommended setup runs both the dev server and admin panel:

```bash
# Start both servers in background
npm run dev:bg
npm run admin:bg

# Open in browser
# Admin Panel: http://localhost:4444
# Gallery Preview: http://localhost:4321

# Stop when done
npm run stop:dev
npm run stop:admin
```

!!! tip "Admin Panel"
    The [Admin Panel](../guides/admin-panel.md) provides a web-based interface for managing albums, photos, and home page content. It's optional but makes content management much easier.

---

## Production Deployment

### Build for Production

```bash
npm run build
```

This creates a `dist/` folder with your optimized site.

### Server Requirements

Since Goldplated Photos uses SSR for password protection, you need a Node.js server:

```bash
# Production start
node dist/server/entry.mjs
```

### Environment Variables

```bash
# Optional: Set production port
PORT=3000
HOST=0.0.0.0
```

---

## Optional: Video Support

For video metadata extraction, install ffprobe:

=== "macOS"
    ```bash
    brew install ffmpeg
    ```

=== "Ubuntu/Debian"
    ```bash
    sudo apt install ffmpeg
    ```

=== "Windows"
    Download from [ffmpeg.org](https://ffmpeg.org/download.html) and add to PATH.

---

## Troubleshooting Installation

### Port Already in Use

The dev server will automatically try ports 4321, 4322, or 4323.

### Sharp Build Errors

Sharp (image processing) may need platform-specific binaries:

```bash
npm rebuild sharp
```

### Thumbnail Permissions

Ensure the `.meta/thumbnails/` directory is writable:

```bash
mkdir -p .meta/thumbnails
chmod 755 .meta/thumbnails
```

---

## Next Steps

- [Quick Start Guide](quick-start.md) - Create your first album
- [Configuration](configuration.md) - Customize your gallery
