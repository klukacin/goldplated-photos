# Thumbnail System

On-demand thumbnail generation with automatic caching.

---

## Overview

Goldplated Photos generates optimized thumbnails using [Sharp](https://sharp.pixelplumbing.com/), a high-performance image processing library.

**Key Features:**

- Three size presets for different uses
- On-demand generation (first request triggers creation)
- Persistent file-based caching
- Automatic EXIF orientation correction

---

## Thumbnail Sizes

| Size | Max Width | Use Case |
|------|-----------|----------|
| `small` | 400px | Grid thumbnails |
| `medium` | 1200px | Lightbox preview |
| `large` | 1920px | Lightbox full view |

All thumbnails maintain aspect ratio and are optimized as progressive JPEGs.

---

## How It Works

### 1. Request Flow

```
Browser → /api/thumbnail?path=photo.jpg&size=small
         ↓
API checks cache (.meta/thumbnails/small/{hash}.jpg)
         ↓
If cached: Serve immediately
If not: Generate → Cache → Serve
```

### 2. Cache Location

```
.meta/thumbnails/
├── small/
│   ├── a1b2c3d4.jpg
│   └── ...
├── medium/
│   └── ...
└── large/
    └── ...
```

Filenames are MD5 hashes of the original path for uniqueness.

### 3. EXIF Orientation

Sharp automatically rotates images based on EXIF orientation metadata:

```typescript
sharp(buffer)
  .rotate()  // Auto-rotate based on EXIF
  .resize(maxWidth, null, { withoutEnlargement: true })
  .jpeg({ quality: 85, progressive: true })
```

!!! info "Why Thumbnails Matter"
    Original images may display rotated incorrectly because browsers don't consistently respect EXIF orientation. Thumbnails are pre-rotated during generation, guaranteeing correct display.

---

## API Endpoint

### GET /api/thumbnail

**Query Parameters:**

| Param | Required | Description |
|-------|----------|-------------|
| `path` | Yes | Relative path to original image |
| `size` | Yes | Size preset: `small`, `medium`, `large` |

**Example:**

```
/api/thumbnail?path=2025/vacation/beach.jpg&size=medium
```

**Response:**

- `200` - JPEG image with caching headers
- `400` - Missing parameters
- `404` - Image not found
- `500` - Processing error

**Cache Headers:**

```
Cache-Control: public, max-age=31536000, immutable
```

---

## Regenerating Thumbnails

To force thumbnail regeneration:

```bash
# Delete entire cache
rm -rf .meta/thumbnails

# Delete specific size
rm -rf .meta/thumbnails/small

# Delete single thumbnail (find hash first)
rm .meta/thumbnails/small/a1b2c3d4.jpg
```

Thumbnails regenerate automatically on next request.

---

## Configuration

### Changing Sizes

Edit `THUMBNAIL_SIZES` in `src/pages/api/thumbnail.ts`:

```typescript
const THUMBNAIL_SIZES = {
  small: 400,   // Grid
  medium: 1200, // Preview
  large: 1920   // Full
};
```

### Changing Quality

Modify the Sharp JPEG options:

```typescript
.jpeg({
  quality: 85,        // 0-100, higher = larger files
  progressive: true   // Progressive loading
})
```

### Changing Format

For WebP output:

```typescript
.webp({ quality: 80 })
```

Update the response content-type accordingly.

---

## Performance

### Generation Speed

- Small (400px): ~50-100ms
- Medium (1200px): ~100-200ms
- Large (1920px): ~200-400ms

Times vary based on original image size and server hardware.

### Disk Usage

Approximate thumbnail sizes (from 5000px original):

| Size | Typical File Size |
|------|-------------------|
| Small | 30-80 KB |
| Medium | 150-300 KB |
| Large | 300-600 KB |

### Memory Usage

Sharp processes images efficiently, but generating many thumbnails simultaneously can spike memory. The API processes one request at a time per-path.

---

## Supported Formats

**Input formats:**

- JPEG (.jpg, .jpeg)
- PNG (.png)
- GIF (.gif) - first frame only
- WebP (.webp)
- HEIC/HEIF (.heic, .heif)

**Output format:**

- JPEG (progressive)

---

## Cover Photos

Album covers use the thumbnail system:

1. **Manual:** Set `thumbnail: "filename.jpg"` in album's `index.md`
2. **Auto:** First photo in the album alphabetically
3. **Fallback:** Generic icon for empty albums

Cover photos use the `medium` size preset.

---

## Troubleshooting

### Thumbnails Not Generating

1. Check `.meta/thumbnails` directory exists and is writable
2. Verify Sharp is installed: `npm ls sharp`
3. Check server logs for errors

### Wrong Orientation

Thumbnails should auto-rotate. If not:

1. Verify original has EXIF orientation data
2. Delete cached thumbnail and regenerate
3. Ensure Sharp's `.rotate()` is called without parameters

### Slow Generation

1. Check available disk space
2. Consider SSD storage for `.meta/` directory
3. Pre-warm cache by visiting albums

---

## Related

- [Photo Grid](photo-grid.md) - How thumbnails are displayed
- [API Endpoints](../reference/api-endpoints.md) - Full API reference
