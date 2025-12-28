# Keyboard Shortcuts

Complete reference for all keyboard shortcuts.

---

## Gallery View

Shortcuts available when browsing the photo grid (before opening lightbox).

| Key | Action |
|-----|--------|
| `Left Arrow` | Move to previous photo |
| `Right Arrow` | Move to next photo |
| `Up Arrow` | Move to photo above |
| `Down Arrow` | Move to photo below |
| `Enter` | Open focused photo in lightbox |
| `Space` | Open focused photo in lightbox |
| `i` | Toggle EXIF info overlay |
| `Home` | Jump to first photo |
| `End` | Jump to last photo |

### Navigation Behavior

Arrow key navigation is spatial - it moves based on visual position in the grid, not just list order.

- **Left/Right** - Move within the current row, wrap to next/previous row
- **Up/Down** - Move to the nearest photo in the row above/below
- Position tolerance of 80px for row detection

### Visual Focus

The currently focused photo displays a blue outline (`.keyboard-focused` class). This helps track your position when using keyboard navigation.

---

## Lightbox View

Shortcuts available when viewing photos in fullscreen lightbox.

### Navigation

| Key | Action |
|-----|--------|
| `Left Arrow` | Previous photo |
| `Right Arrow` | Next photo |
| `Home` | First photo |
| `End` | Last photo |

### Display Modes

| Key | Action |
|-----|--------|
| `F` / `f` | Toggle fullscreen |
| `O` / `o` | Toggle original quality (full resolution) |
| `H` / `h` | Toggle zen mode (hide all UI) |
| `I` / `i` | Toggle photo info (EXIF) overlay |

### Help & Exit

| Key | Action |
|-----|--------|
| `?` | Toggle keyboard shortcuts help |
| `Escape` | Close (layered: help → EXIF → lightbox) |

### Escape Key Behavior

The Escape key closes overlays in order of precedence:

1. **First press** - Close shortcuts help overlay (if open)
2. **Second press** - Close EXIF info overlay (if open)
3. **Third press** - Close the lightbox

This layered behavior prevents accidentally closing the lightbox when dismissing an overlay.

---

## Mode Descriptions

### Original Quality Mode (`O`)

- **Off (default):** Images load at 1920px max (fast)
- **On:** Images load at full original resolution

Use when you need to inspect fine details or verify image quality.

### Zen Mode (`H`)

- Hides all UI elements (toolbar, counter, buttons)
- Mouse hover near edges reveals UI temporarily
- Perfect for presentations or distraction-free viewing

### EXIF Info Overlay (`I`)

Displays camera metadata:

- Camera make and model
- Lens information
- Focal length, aperture, shutter speed, ISO
- Date taken
- Image dimensions

---

## Touch Gestures

### In Lightbox

| Gesture | Action |
|---------|--------|
| Swipe left | Next photo |
| Swipe right | Previous photo |
| Swipe up/down | Close lightbox |
| Pinch | Zoom in/out |
| Double-tap | Toggle zoom |

### In Gallery (Mobile)

| Gesture | Action |
|---------|--------|
| Tap | Open photo in lightbox |
| Long-press (500ms) | Open context menu |

---

## Accessibility

### Screen Reader Support

- All interactive elements have ARIA labels
- Image counter announces current position
- Keyboard navigation respects focus order

### Focus Indicators

- Visible focus outlines on all interactive elements
- High contrast focus rings (blue, 3px)
- Focus trapped within lightbox when open

### Reduced Motion

When `prefers-reduced-motion: reduce` is set:

- Transitions are disabled
- Animations are minimized
- All functionality remains accessible

---

## Quick Reference Card

```
╔═══════════════════════════════════════════╗
║           KEYBOARD SHORTCUTS              ║
╠═══════════════════════════════════════════╣
║  Gallery View                             ║
║    ← → ↑ ↓    Navigate photos             ║
║    Enter      Open in lightbox            ║
║    i          Photo info                  ║
╠═══════════════════════════════════════════╣
║  Lightbox View                            ║
║    ← →        Previous/Next               ║
║    F          Fullscreen                  ║
║    O          Original quality            ║
║    H          Zen mode (hide UI)          ║
║    I          Photo info (EXIF)           ║
║    ?          Show shortcuts              ║
║    Escape     Close                       ║
╚═══════════════════════════════════════════╝
```

---

## Related

- [Photo Grid](../features/photo-grid.md) - Gallery features
- [Lightbox](../features/lightbox.md) - Lightbox features
