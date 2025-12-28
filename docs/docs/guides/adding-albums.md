# Adding Albums

Complete guide to creating and organizing photo albums.

---

## Album Structure

Albums are folders in `src/content/albums/` with:

1. An `index.md` file (required)
2. Photo/video files

```
src/content/albums/
└── 2025/
    └── events/
        └── birthday-party/
            ├── index.md      # Album metadata
            ├── cake.jpg
            ├── group.jpg
            └── dance.mp4
```

---

## Creating an Album

### Step 1: Create the Folder

```bash
mkdir -p src/content/albums/2025/events/birthday-party
```

### Step 2: Add index.md

Create `index.md` with at minimum:

```yaml
---
title: "Birthday Party"
---
```

### Step 3: Add Photos

Copy your photos into the folder. Supported formats:

- **Images:** .jpg, .jpeg, .png, .gif, .webp, .heic, .heif
- **Videos:** .mp4, .mov, .webm

### Step 4: View Your Album

Start the dev server and navigate to `/photos/2025/events/birthday-party`

---

## Album Options

### Full Configuration

```yaml
---
title: "Birthday Party"
thumbnail: "cake.jpg"       # Cover photo
password: "secret123"       # Password protection
order: 1                    # Sort order (lower = first)
hidden: false               # Hide from listings
isCollection: false         # Contains sub-albums instead of photos
allowAnonymous: false       # Allow token-based sharing
style: "grid"               # Default view: grid, masonry, single
---
```

### Setting a Cover Photo

By default, the first photo alphabetically becomes the cover. To choose:

```yaml
---
title: "Birthday Party"
thumbnail: "best-moment.jpg"
---
```

### Ordering Albums

Control the display order in listings:

```yaml
---
title: "Featured Album"
order: 1  # Appears first
---

---
title: "Second Album"
order: 2  # Appears second
---
```

Albums without `order` appear after ordered albums, sorted alphabetically.

### Hidden Albums

Hide from listings but keep accessible via direct URL:

```yaml
---
title: "Private Album"
hidden: true
---
```

Access via: `/photos/2025/events/private-album`

---

## Creating Collections

Collections are albums that contain other albums (no photos directly).

### Collection Structure

```
src/content/albums/
└── 2025/
    └── weddings/              # Collection
        ├── index.md           # isCollection: true
        ├── john-and-mary/     # Album
        │   ├── index.md
        │   └── *.jpg
        └── alex-and-sam/      # Album
            ├── index.md
            └── *.jpg
```

### Collection index.md

```yaml
---
title: "2025 Weddings"
isCollection: true
thumbnail: "john-and-mary/ceremony.jpg"  # Can reference sub-album photo
---
```

Collections display as a grid of sub-album covers.

---

## Recommended Organization

### By Year and Category

```
src/content/albums/
├── 2024/
│   ├── weddings/
│   ├── events/
│   └── travel/
└── 2025/
    ├── weddings/
    ├── events/
    └── travel/
```

### By Event Type

```
src/content/albums/
├── weddings/
│   ├── 2024-john-mary/
│   └── 2025-alex-sam/
├── portraits/
└── landscape/
```

---

## Naming Conventions

!!! warning "Avoid Dots After Numbers"
    Astro normalizes folder names, breaking paths with dots.

    - `16.album-name` becomes `16album-name`
    - Use dashes: `16-album-name`

### Good Names

- `summer-vacation`
- `2025-01-wedding`
- `johns-birthday-party`

### Avoid

- `16.album` (dot after number)
- `album name` (spaces)
- `album/name` (slashes)

---

## Bulk Import

### Script Example

```bash
#!/bin/bash
# Import photos from a folder

SOURCE="/path/to/photos"
DEST="src/content/albums/2025/imported"

mkdir -p "$DEST"

# Create index.md
cat > "$DEST/index.md" << EOF
---
title: "Imported Photos"
---
EOF

# Copy photos
cp "$SOURCE"/*.{jpg,jpeg,png} "$DEST/" 2>/dev/null
```

### Preserving Dates

Photos retain their EXIF dates. Sorting by "Date (Oldest)" uses EXIF data, not file system dates.

---

## Photo Requirements

### Recommended

- JPEG format for best compatibility
- Long edge: 3000-6000px for high quality
- Include EXIF data (camera, date, settings)

### Size Limits

No hard limits, but consider:

- Very large files (50MB+) slow thumbnail generation
- Thumbnails are generated at 400px, 1200px, 1920px

### Orientation

Thumbnails auto-rotate based on EXIF. Original images may display incorrectly in some contexts, but the gallery always uses correctly-oriented thumbnails.

---

## Troubleshooting

### Album Not Appearing

1. Verify `index.md` exists in the album folder
2. Check for YAML syntax errors in frontmatter
3. Restart the dev server

### Photos Not Loading

1. Check file extensions are lowercase
2. Verify files aren't corrupted
3. Check browser console for errors

### Wrong Cover Photo

1. Set `thumbnail: "filename.jpg"` explicitly
2. Ensure the filename matches exactly (case-sensitive)

---

## Related

- [Configuration](../getting-started/configuration.md) - All options
- [Password Protection](../features/password-protection.md) - Private albums
- [Thumbnails](../features/thumbnails.md) - Image processing
