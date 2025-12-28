# Customization

Personalize the look and feel of your gallery.

---

## Typography

### Changing Fonts

Fonts are loaded in `src/layouts/Layout.astro`:

```html
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Syne:wght@500;600;700;800&family=DM+Sans:wght@400;500;600&family=Satisfy&display=swap" rel="stylesheet">
```

Update the CSS variables:

```css
:root {
  --font-heading: 'Syne', sans-serif;
  --font-body: 'DM Sans', sans-serif;
  --font-signature: 'Satisfy', cursive;
}
```

### Font Scale

The site uses fluid typography with `clamp()`:

```css
--text-xs: clamp(0.75rem, 0.7rem + 0.25vw, 0.875rem);
--text-sm: clamp(0.875rem, 0.8rem + 0.35vw, 1rem);
--text-base: clamp(1rem, 0.9rem + 0.5vw, 1.125rem);
--text-lg: clamp(1.125rem, 1rem + 0.6vw, 1.25rem);
--text-xl: clamp(1.25rem, 1.1rem + 0.75vw, 1.5rem);
--text-2xl: clamp(1.5rem, 1.25rem + 1.25vw, 2rem);
--text-3xl: clamp(2rem, 1.5rem + 2.5vw, 3rem);
--text-4xl: clamp(2.5rem, 2rem + 2.5vw, 4rem);
```

---

## Colors

### Color Tokens

```css
:root {
  /* Text colors */
  --color-text: #0a0a0a;           /* Primary text */
  --color-text-secondary: #404040;  /* Secondary text */
  --color-text-muted: #666666;      /* Muted text */

  /* UI colors */
  --color-focus: #2563eb;           /* Focus rings */
  --color-border: #e5e5e5;          /* Borders */
  --color-bg: #ffffff;              /* Backgrounds */
}
```

### Dark Mode

To add dark mode, use CSS custom properties with `prefers-color-scheme`:

```css
@media (prefers-color-scheme: dark) {
  :root {
    --color-text: #f5f5f5;
    --color-text-secondary: #a0a0a0;
    --color-bg: #0a0a0a;
  }
}
```

---

## Spacing

### Spacing Scale

```css
--space-xs: 0.5rem;   /* 8px */
--space-sm: 1rem;     /* 16px */
--space-md: 1.5rem;   /* 24px */
--space-lg: 2rem;     /* 32px */
--space-xl: 3rem;     /* 48px */
--space-2xl: 5rem;    /* 80px */
--space-3xl: 8rem;    /* 128px */
```

### Container Widths

```css
--container-narrow: 700px;
--container-max: 1000px;
--container-wide: 1200px;
```

---

## Layout Components

### Landing Page

Edit `src/pages/index.astro`:

- Background image: `public/images/landing-bg.jpg`
- Shutter sound: `public/sounds/shutter.mp3`

### Home Page

Edit `src/pages/home.astro`:

- Hero images: `public/home/hero/*.jpg`
- Content cards: `src/content/home/cards/*.md`
- Intro text: `src/content/home/intro.md`

### Photo Grid

Modify `src/components/PhotoGrid.astro`:

- Grid columns and gaps
- Hover effects
- View mode layouts

---

## Content Cards

Cards on the home page are markdown files in `src/content/home/cards/`:

```yaml
---
type: "card"
title: "Weddings"
image: "/home/cards/weddings.jpg"
imagePosition: "left"  # or "right"
link: "/photos/weddings"
order: 1
---

Beautiful wedding photography capturing your special moments.
```

### Image Position

- `left` - Image on left, text on right
- `right` - Image on right, text on left

Cards alternate automatically based on order.

---

## Footer

Edit `src/components/Footer.astro`:

```astro
<footer>
  <p>
    Photos by <span class="signature">Your Name</span>
  </p>
  <p class="contact">
    <a href="mailto:your@email.com">Contact</a>
  </p>
</footer>
```

---

## Album Styles

Set default view mode per album:

```yaml
---
title: "Portrait Gallery"
style: "single"  # grid, masonry, or single
---
```

- `grid` - Square thumbnails (default)
- `masonry` - Variable height, Pinterest-style
- `single` - Full-width images

---

## Lightbox Customization

### Custom Toolbar

The lightbox toolbar is created dynamically in `PhotoGrid.astro`. To modify:

1. Find the `uiRegister` event handler
2. Edit the HTML structure
3. Update corresponding CSS (use `:global()` selectors)

### Colors

```css
:global(.pswp__bg) {
  background: rgba(0, 0, 0, 0.95);
}

:global(.pswp__counter) {
  color: #ffffff;
}
```

---

## Accessibility

### Focus Indicators

```css
:focus-visible {
  outline: 3px solid var(--color-focus);
  outline-offset: 2px;
}
```

### Reduced Motion

Animations respect user preferences:

```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
  }
}
```

### High Contrast

```css
@media (forced-colors: active) {
  /* Styles for Windows High Contrast mode */
}
```

---

## Adding Custom CSS

Create a custom stylesheet and import it in `Layout.astro`:

```astro
---
// src/layouts/Layout.astro
---
<head>
  <link rel="stylesheet" href="/styles/custom.css">
</head>
```

---

## Related

- [Configuration](../getting-started/configuration.md) - All options
- [UI Components](../reference/ui-components.md) - Component details
