# Photo Grid

The main photo display component with multiple view modes, keyboard navigation, and sorting options.

---

## View Modes

### Grid View

Square thumbnails in a responsive grid layout.

- Uniform appearance
- Maximum photos visible
- Best for browsing large albums

### Masonry View

Pinterest-style variable-height layout preserving aspect ratios.

- True left-to-right ordering (CSS Grid)
- Desktop: 3 columns
- Mobile: 2 columns

### Single Column View

Full-width images with original aspect ratios.

- Best for viewing individual photos
- Original proportions preserved
- Ideal for portrait photography

---

## Photo Actions

### Desktop (Hover Icons)

Hover over any photo to reveal action icons in the top-right corner:

- **Info icon** - Show EXIF/metadata overlay
- **Download icon** - Download the photo

**Icon Design:**

- 36px diameter circular buttons
- Semi-transparent white background
- Subtle shadow and backdrop blur
- Scale 1.1x on hover

### Mobile (Long-Press Menu)

On mobile devices (< 768px), icons are hidden. Instead:

1. **Press and hold** any photo for 500ms
2. **Haptic feedback** vibration occurs
3. **Context menu** appears at touch position
4. Options: "Photo Info" and "Download"

---

## Sorting Options

Six sort modes available via dropdown (persisted to localStorage):

| Option | Description |
|--------|-------------|
| Name (A-Z) | Alphabetical ascending |
| Name (Z-A) | Alphabetical descending |
| Date (Oldest) | By EXIF date, oldest first (default) |
| Date (Newest) | By EXIF date, newest first |
| Size (Smallest) | By file size ascending |
| Size (Largest) | By file size descending |

---

## Keyboard Navigation

### Gallery View

| Key | Action |
|-----|--------|
| Arrow keys | Navigate photos (spatial, row-aware) |
| Enter / Space | Open focused photo in lightbox |
| `i` | Toggle EXIF overlay |
| Home | Jump to first photo |
| End | Jump to last photo |

### Visual Focus

Active photo has a blue outline (`.keyboard-focused` class).

Navigation is spatial - pressing Right moves to the next photo in the visual row, respecting the grid layout.

---

## Video Support

Videos are displayed inline with:

- Native HTML5 player controls
- Poster frame from first frame
- Video info overlay (duration, codec, dimensions)
- Error state with filename and download fallback

### Supported Formats

- `.mp4` (H.264)
- `.mov`
- `.webm`

---

## EXIF Display

Press `i` or click the info icon to show:

- Camera make and model
- Lens information
- Focal length
- Aperture (f-stop)
- Shutter speed
- ISO sensitivity
- Date taken
- Image dimensions

EXIF data is cached per session to avoid repeated API calls.

---

## Technical Details

### Component: `PhotoGrid.astro`

Location: `src/components/PhotoGrid.astro`

**Features:**

- PhotoSwipe lightbox integration
- Thumbnail URL generation via `getThumbnailUrl()`
- In-memory EXIF cache (`exifCache` Map)
- Event capture phase for keyboard handling
- Responsive breakpoints

### CSS

- Grid: `display: grid` with `auto-fill`
- Masonry: `grid-template-columns: repeat(3, 1fr)`
- Photos scale with `object-fit: cover`

### Accessibility

- All images have `alt` text (filename)
- Tab-navigable with focus indicators
- ARIA labels on action buttons
- Keyboard shortcuts work alongside PhotoSwipe

---

## Related

- [Lightbox](lightbox.md) - Full-screen viewing
- [Keyboard Shortcuts](../reference/keyboard-shortcuts.md) - All shortcuts
- [UI Components](../reference/ui-components.md) - Design details
