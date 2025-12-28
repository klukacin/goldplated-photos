# UI Components

Detailed reference for UI components and interactions.

---

## Photo Cards

### Desktop Experience

On hover, photos reveal action icons in the top-right corner:

**Icon Buttons:**

- **Info icon** (circle-i) - Shows EXIF/metadata overlay
- **Download icon** (arrow-down) - Downloads the photo

**Design Specifications:**

| Property | Value |
|----------|-------|
| Size | 36px diameter |
| Background | White, 95% opacity |
| Shadow | Subtle drop shadow |
| Blur | backdrop-filter: blur(10px) |
| Hover | Scale 1.1x, primary color |

**CSS:**

```css
.icon-btn {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(10px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
}

.icon-btn:hover {
  transform: scale(1.1);
  color: var(--color-focus);
}
```

### Mobile Experience

On mobile devices (< 768px), hover icons are hidden. Instead:

**Long-Press Context Menu:**

1. Press and hold any photo for 500ms
2. Haptic feedback vibration occurs
3. Context menu appears at touch position
4. Options: "Photo Info" and "Download"

**Menu Design:**

| Property | Value |
|----------|-------|
| Background | White |
| Shadow | Large, elevated |
| Border radius | 12px |
| Animation | Scale-in (0.2s ease-out) |
| Touch targets | 48px height minimum |

---

## View Mode Selector

Dropdown menu to switch between layouts.

**Options:**

| Mode | Icon | Description |
|------|------|-------------|
| Grid | ⊞ | Square thumbnails |
| Masonry | ⊟ | Variable heights |
| Single | ▢ | Full-width images |

**Persistence:**

Selection saved to localStorage key: `photoGallery_viewMode`

---

## Sort Selector

Dropdown menu for photo sorting.

**Options:**

| Option | Description |
|--------|-------------|
| Name (A-Z) | Alphabetical ascending |
| Name (Z-A) | Alphabetical descending |
| Date (Oldest) | EXIF date, oldest first |
| Date (Newest) | EXIF date, newest first |
| Size (Smallest) | File size ascending |
| Size (Largest) | File size descending |

**Default:** Date (Oldest)

**Persistence:**

Selection saved to localStorage key: `photoGallery_sortOption`

---

## Lightbox UI

Custom toolbar elements added to PhotoSwipe.

### Top Bar Left

**Image Counter:**

- Format: "1 / 24"
- Includes thumbnail preview on hover
- Updates in real-time during navigation

### Top Bar Right

| Element | Description |
|---------|-------------|
| Size display | Current image dimensions |
| Original button | Toggle full resolution (🔍+) |
| Help button | Show keyboard shortcuts (?) |
| Close button | Exit lightbox (×) |

### Overlays

**EXIF Info Overlay:**

- Semi-transparent dark background
- Camera and shooting info
- Close with `I` key or click outside

**Shortcuts Help Overlay:**

- Full-screen semi-transparent
- Grouped by category
- Close with `?` key or Escape

---

## Password Form

Displayed for protected albums when not authenticated.

**Elements:**

- Album title
- Password input field
- Submit button
- Error message area
- Rate limit warning

**Error States:**

| Error | Message |
|-------|---------|
| `wrong-password` | "Incorrect password" + remaining attempts |
| `rate-limited` | "Too many attempts. Please try again in 15 minutes." |
| `missing-fields` | "Please enter a password" |

---

## Breadcrumbs

Hierarchical navigation path.

**Format:**

```
Home / Photos / 2025 / Weddings / John & Mary
```

**Behavior:**

- Each segment is a clickable link
- Current page is not linked (plain text)
- Truncates on mobile with ellipsis

---

## Album Grid

Grid of album cards with cover photos.

**Card Elements:**

- Cover photo thumbnail (medium size)
- Album title
- Photo count badge (optional)
- Lock icon (if password-protected)

**Responsive:**

| Breakpoint | Columns |
|------------|---------|
| Mobile | 2 |
| Tablet | 3 |
| Desktop | 4 |

---

## Hero Slider

Auto-rotating image carousel on the home page.

**Features:**

- Auto-advances every 5 seconds
- Pause on hover/focus
- Keyboard navigation (arrows)
- Dot indicators for direct access
- Respects reduced motion preference

**ARIA Attributes:**

```html
<div role="region" aria-roledescription="carousel" aria-label="Featured photos">
  <div role="group" aria-roledescription="slide" aria-label="1 of 3">
    ...
  </div>
</div>
```

---

## Content Cards

Alternating image/text blocks on the home page.

**Layout:**

- Image on left, text on right (or reversed)
- Alternates based on order
- Full-width on mobile

**Entry Animation:**

- Fade-in and slide-up on scroll
- Staggered timing per card
- Disabled when reduced motion preferred

---

## Footer

Simple site footer.

**Elements:**

- Signature line ("Photos by Kristijan Lukacin")
- Contact email link
- Copyright notice

**Typography:**

- Signature uses Satisfy font (cursive)
- Contact uses muted text color

---

## Focus States

All interactive elements have visible focus indicators.

**Default Focus:**

```css
:focus-visible {
  outline: 3px solid var(--color-focus);
  outline-offset: 2px;
}
```

**Keyboard Focus on Photos:**

```css
.photo-item.keyboard-focused {
  outline: 3px solid var(--color-focus);
  outline-offset: 3px;
}
```

---

## Touch Targets

Minimum touch target sizes per WCAG guidelines:

| Element | Size |
|---------|------|
| Icon buttons | 36px (+ padding = 44px) |
| Menu items | 48px height |
| Slider dots | 44px tap area |
| Links | 44px minimum height |

---

## Related

- [Photo Grid](../features/photo-grid.md) - Grid features
- [Lightbox](../features/lightbox.md) - Lightbox features
- [Customization](../guides/customization.md) - Styling guide
