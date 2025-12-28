# Quick Start

Get your photo gallery running in under 5 minutes.

---

## 1. Start the Server

```bash
npm run dev
```

Visit `http://localhost:4321` to see your gallery.

---

## 2. Create Your First Album

Create a folder structure in `src/content/albums/`:

```
src/content/albums/
└── 2025/
    └── vacation/
        └── beach-trip/
            ├── index.md
            ├── photo1.jpg
            ├── photo2.jpg
            └── photo3.jpg
```

### Album Metadata (index.md)

```yaml
---
title: "Beach Trip"
---
```

That's it! Refresh the gallery and your album appears.

---

## 3. Add More Options

### Set a Cover Photo

```yaml
---
title: "Beach Trip"
thumbnail: "sunset.jpg"
---
```

### Add Password Protection

```yaml
---
title: "Private Album"
password: "secret123"
---
```

### Control Display Order

```yaml
---
title: "Featured Album"
order: 1  # Lower numbers appear first
---
```

### Hide from Listings

```yaml
---
title: "Hidden Album"
hidden: true  # Only accessible via direct URL
---
```

---

## 4. Supported File Types

### Photos
- `.jpg`, `.jpeg`
- `.png`
- `.gif`
- `.webp`
- `.heic`, `.heif`

### Videos
- `.mp4`
- `.mov`
- `.webm`

---

## 5. View Modes

Switch between layouts using the view mode selector:

| Mode | Description |
|------|-------------|
| Grid | Square thumbnails in a uniform grid |
| Masonry | Pinterest-style variable heights |
| Single | Full-width images, original aspect ratios |

---

## 6. Keyboard Shortcuts

### In Gallery View
| Key | Action |
|-----|--------|
| Arrow keys | Navigate photos |
| Enter/Space | Open lightbox |
| `i` | Show photo info |

### In Lightbox
| Key | Action |
|-----|--------|
| Left/Right | Navigate |
| `i` | Toggle EXIF info |
| `f` | Toggle fullscreen |
| `o` | Original quality |
| `h` | Zen mode (hide UI) |
| `?` | Show all shortcuts |
| Escape | Close |

---

## 7. Create Collections

Collections are albums that contain other albums (no photos):

```
src/content/albums/
└── 2025/
    └── weddings/           # Collection
        ├── index.md        # isCollection: true
        ├── john-mary/      # Album
        │   ├── index.md
        │   └── *.jpg
        └── alex-sam/       # Album
            ├── index.md
            └── *.jpg
```

```yaml
# src/content/albums/2025/weddings/index.md
---
title: "Weddings"
isCollection: true
---
```

---

## Next Steps

- [Configuration](configuration.md) - All available options
- [Adding Albums](../guides/adding-albums.md) - Detailed album guide
- [Password Protection](../features/password-protection.md) - Security features
