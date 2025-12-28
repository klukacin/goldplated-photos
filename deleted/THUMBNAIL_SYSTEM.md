# Thumbnail Generation System

## Overview

The photo gallery now uses an intelligent thumbnail generation system that reduces bandwidth usage by ~99.9% for grid views.

## How It Works

### Automatic Generation
- When a photo is first requested, a thumbnail is generated using Sharp
- Thumbnails are cached in each album's `.meta/thumbnails/` directory
- Subsequent requests serve the cached version instantly

### Three Sizes Available

1. **Small (400px)** - Used for grid thumbnails
2. **Medium (1200px)** - For lightbox preview
3. **Large (1920px)** - For full-screen viewing

### Performance Benefits

**Before (Full-Resolution)**:
- Original photo: ~28 MB
- Grid displays at: ~280px width
- Bandwidth waste: ~95%

**After (Thumbnails)**:
- Thumbnail: ~25 KB
- Grid displays at: ~280px width
- **Bandwidth savings: 99.9%**

**For 10 photos**:
- Before: 280 MB download
- After: 250 KB download
- **Total savings: ~279.75 MB per page load**

## Cache Management

### Location
Thumbnails are cached **per-album** in each album's `.meta` directory:

```
src/content/albums/
└── 2025/
    └── Birthdays/
        └── Marias-Birthday/
            ├── index.md
            ├── photo1.jpg          # Original (28 MB)
            ├── photo2.jpg
            └── .meta/              # Cache directory (git-ignored)
                └── thumbnails/
                    ├── small/
                    │   ├── photo1.jpg  # Thumbnail (27 KB)
                    │   └── photo2.jpg
                    ├── medium/
                    │   └── ...
                    └── large/
                        └── ...
```

### Regenerate Thumbnails

To force regeneration (e.g., after updating photos):

```bash
# Delete all thumbnails for a specific album
rm -rf src/content/albums/2025/Birthdays/Marias-Birthday/.meta/thumbnails

# Or delete specific size
rm -rf src/content/albums/2025/Birthdays/Marias-Birthday/.meta/thumbnails/small

# Or delete all thumbnails across all albums
find src/content/albums -type d -name ".meta" -exec rm -rf {} +
```

Thumbnails will be automatically regenerated on next request.

### Git Ignore

The `.meta/` directory is automatically ignored by git (added to `.gitignore`).

## API Endpoint

### Usage

```
GET /api/thumbnail?path=<photo-path>&size=<size>
```

**Parameters**:
- `path` - Photo path relative to `src/content/albums/` (required)
- `size` - One of: `small`, `medium`, `large` (default: `small`)

**Example**:
```
/api/thumbnail?path=2025/Birthdays/Marias-Birthday/photo.jpg&size=small
```

### Response

- **Headers**: `Cache-Control: public, max-age=31536000, immutable`
- **Content-Type**: `image/jpeg`
- **Status**: 200 (success) or 500 (fallback to original)

### Fallback Behavior

If thumbnail generation fails, the API automatically falls back to serving the original image.

## Technical Details

### Sharp Configuration

```typescript
sharp(sourcePath)
  .resize(width, null, {
    width: width,
    withoutEnlargement: true,
    fit: 'inside'
  })
  .jpeg({
    quality: 85,
    progressive: true
  })
```

### Cache File Naming

Thumbnails use the **same filename** as the original photo:
- Original: `src/content/albums/2025/Album/photo.jpg`
- Thumbnail: `src/content/albums/2025/Album/.meta/thumbnails/small/photo.jpg`
- Easy to identify and manage
- Preserves original file extension

### Security

- Path traversal prevention (`..` and leading `/` blocked)
- Size validation (only `small`, `medium`, `large` allowed)
- Original file existence check before processing

## Integration with PhotoGrid

The `PhotoGrid.astro` component automatically uses thumbnails:

```astro
<img
  src={getThumbnailUrl(photo.url, 'small')}
  alt={photo.filename}
  loading="lazy"
/>
```

## Browser Caching

Thumbnails are cached by browsers for 1 year (`max-age=31536000`), ensuring:
- Instant page loads on subsequent visits
- Minimal server bandwidth
- Optimal performance

## Disk Space Usage

**Typical usage for 100 photos**:
- Originals: ~2.8 GB
- Small thumbnails: ~2.5 MB
- Medium thumbnails: ~15 MB
- Large thumbnails: ~50 MB
- **Total cache: ~67.5 MB (2.4% of originals)**

## When to Regenerate

Regenerate thumbnails when:
- You replace photos with updated versions
- You change Sharp quality/resize settings
- Thumbnails appear corrupted

**Commands**:
```bash
# For a specific album
rm -rf src/content/albums/2025/Birthdays/Marias-Birthday/.meta/thumbnails

# For all albums
find src/content/albums -type d -name ".meta" -exec rm -rf {} +
```

Then visit your gallery - thumbnails regenerate automatically on first view.

## Monitoring

Check thumbnail generation in server logs:

```
[Thumbnail] Cache miss: 2025/Birthdays/photo.jpg (small)
[Thumbnail] Generated and cached: 2025/Birthdays/photo.jpg (small)
[Thumbnail] Cache hit: 2025/Birthdays/photo.jpg (small)
```

## Production Deployment

### Build Time
- `.meta/` folder should NOT be committed to git
- Thumbnails generate on-demand in production
- First visitor generates thumbnails, subsequent visitors use cache

### Recommended Setup
1. Deploy to host with persistent filesystem (e.g., VPS, dedicated server)
2. Or pre-generate thumbnails during build:
   ```bash
   npm run build
   # Visit all album pages to pre-generate thumbnails
   ```
3. Or use CDN caching for `/api/thumbnail` endpoint

## Summary

✅ **99.9% bandwidth reduction**
✅ **Automatic generation and caching**
✅ **Per-album cache organization**
✅ **Simple regeneration**: Delete `.meta/thumbnails` in any album
✅ **Three sizes for different use cases**
✅ **Browser caching for 1 year**
✅ **Fallback to originals if generation fails**

Your photo gallery is now optimized for production!
