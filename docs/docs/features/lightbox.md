# Lightbox

Full-screen photo viewing powered by PhotoSwipe with custom UI enhancements.

---

## Opening the Lightbox

- **Click/tap** any photo in the grid
- **Press Enter/Space** on focused photo
- Photos open at their clicked position with smooth animation

---

## Navigation

### Mouse/Touch

- **Swipe left/right** - Navigate photos
- **Swipe up/down** - Close lightbox (mobile)
- **Pinch** - Zoom in/out
- **Double-tap** - Toggle zoom

### Keyboard

| Key | Action |
|-----|--------|
| Left/Right arrows | Navigate photos |
| Escape | Close (layered: help > EXIF > lightbox) |
| `F` | Toggle fullscreen |

---

## Custom UI Elements

### Top Bar (Left)

- **Image counter** - "1 / 24" with thumbnail preview
- Shows current position in album

### Top Bar (Right)

- **Size display** - Current image dimensions
- **Original button** (magnifier+) - Load full resolution
- **Help button** (?) - Show keyboard shortcuts
- **Close button** (x) - Exit lightbox

---

## Special Modes

### Original Quality Mode

Press `O` to load full-resolution original images instead of thumbnails.

- Thumbnails are 1920px max (fast loading)
- Originals can be 6000px+ (full detail)
- Toggle persists during session

### Zen Mode

Press `H` to hide all UI elements for distraction-free viewing.

- All overlays and buttons hidden
- Hover near edges reveals UI
- Perfect for presentations

### EXIF Overlay

Press `I` to show photo metadata.

- Camera and lens info
- Exposure settings
- Date and dimensions
- Semi-transparent overlay

---

## Keyboard Shortcuts Help

Press `?` to show all available shortcuts:

```
Navigation
  Left/Right  Next/Previous photo
  Home/End    First/Last photo

Display
  F           Toggle fullscreen
  O           Original quality
  H           Zen mode (hide UI)
  I           Photo info (EXIF)

General
  ?           Show this help
  Escape      Close overlay/lightbox
```

---

## Escape Key Behavior

The Escape key closes overlays in order:

1. **First press** - Close shortcuts help (if open)
2. **Second press** - Close EXIF overlay (if open)
3. **Third press** - Close lightbox

This layered approach prevents accidental exits.

---

## Grid Sync

When navigating in the lightbox:

- The gallery grid scrolls to keep the current photo visible
- Visual focus indicator updates in real-time
- On close, the grid is centered on the last viewed photo

---

## PhotoSwipe Features

Built on [PhotoSwipe 5](https://photoswipe.com):

- GPU-accelerated animations
- Touch gesture recognition
- Automatic image preloading
- History API integration (back button works)
- Responsive image switching

---

## CSS Notes

Custom UI elements are dynamically created and appended to PhotoSwipe's container. All styles use `:global()` in Astro to target these elements:

```css
:global(.pswp__custom-counter) { ... }
:global(.pswp__original-button) { ... }
:global(.pswp__zen-mode) { ... }
```

---

## Related

- [Photo Grid](photo-grid.md) - Gallery component
- [Keyboard Shortcuts](../reference/keyboard-shortcuts.md) - All shortcuts
