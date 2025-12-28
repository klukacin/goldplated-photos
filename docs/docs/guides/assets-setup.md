# Assets Setup

Guide to setting up images and sounds for your gallery.

---

## Required Assets

### Landing Page Background

**Location:** `public/images/landing-bg.jpg`

A high-quality background image for the landing page.

**Requirements:**

- Format: JPEG
- Recommended: 1920x1080 or larger
- Should be visually striking

### Shutter Sound

**Location:** `public/sounds/shutter.mp3`

Sound effect for the shutter button on the landing page.

**Requirements:**

- Format: MP3
- Duration: 0.2-0.5 seconds
- Clean, crisp camera shutter sound

**Free Sources:**

- [Freesound.org](https://freesound.org/) - Search "camera shutter"
- [Mixkit](https://mixkit.co/free-sound-effects/camera/)

---

## Home Page Assets

### Hero Slider Images

**Location:** `public/home/hero/`

Images for the rotating hero banner.

**Files:**

- `image1.jpg`
- `image2.jpg`
- `image3.jpg`

**Requirements:**

- Format: JPEG
- Aspect ratio: 16:9 or similar wide format
- Minimum width: 1920px
- High resolution for large displays

### Content Card Images

**Location:** `public/home/cards/`

Images displayed alongside content cards on the home page.

**Files match card markdown:**

- `weddings.jpg`
- `people.jpg`
- `events.jpg`
- `street.jpg`
- etc.

**Requirements:**

- Format: JPEG
- Landscape orientation preferred
- Minimum width: 800px
- Should represent the linked content

---

## Placeholder Files

If running without real assets, the site will work but display broken images. Create placeholder files or use stock photos during development.

### Quick Placeholder Script

```bash
#!/bin/bash
# Create placeholder directories
mkdir -p public/images
mkdir -p public/sounds
mkdir -p public/home/hero
mkdir -p public/home/cards

# Download placeholder images (using Lorem Picsum)
curl -L "https://picsum.photos/1920/1080" -o public/images/landing-bg.jpg
curl -L "https://picsum.photos/1920/800" -o public/home/hero/image1.jpg
curl -L "https://picsum.photos/1920/800" -o public/home/hero/image2.jpg
curl -L "https://picsum.photos/1920/800" -o public/home/hero/image3.jpg
```

---

## Image Optimization

### Before Uploading

Optimize images to reduce file size:

```bash
# Using ImageMagick
mogrify -strip -quality 85 -resize "2000x2000>" public/home/hero/*.jpg

# Using jpegoptim
jpegoptim --max=85 public/home/hero/*.jpg
```

### Recommended Sizes

| Asset | Recommended Size |
|-------|------------------|
| Landing background | 1920x1080 (or larger) |
| Hero slider | 1920x800 |
| Card images | 800x600 |

---

## Directory Structure

```
public/
├── images/
│   └── landing-bg.jpg          # Landing page background
├── sounds/
│   └── shutter.mp3             # Shutter button sound
└── home/
    ├── hero/
    │   ├── image1.jpg          # Hero slider image 1
    │   ├── image2.jpg          # Hero slider image 2
    │   └── image3.jpg          # Hero slider image 3
    └── cards/
        ├── weddings.jpg        # Card image
        ├── people.jpg          # Card image
        └── ...                 # More card images
```

---

## Content Card Configuration

Cards are defined in `src/content/home/cards/`:

```yaml
# src/content/home/cards/weddings.md
---
type: "card"
title: "Weddings"
image: "/home/cards/weddings.jpg"
imagePosition: "left"
link: "/photos/weddings"
order: 1
---

Your description text here.
```

The `image` field should match the filename in `public/home/cards/`.

---

## Customizing the Landing Page

### Changing the Background

1. Replace `public/images/landing-bg.jpg` with your image
2. Optionally adjust overlay opacity in `src/pages/index.astro`

### Changing the Shutter Sound

1. Replace `public/sounds/shutter.mp3`
2. Keep it short (0.2-0.5 seconds) for best UX

### Removing Sound

Edit `src/pages/index.astro` and remove the audio element and play code.

---

## Adding More Hero Slides

1. Add more images to `public/home/hero/` (e.g., `image4.jpg`)
2. The slider automatically picks up all JPG files in the directory

---

## Troubleshooting

### Images Not Loading

1. Check file paths are correct (case-sensitive)
2. Verify files exist in `public/` directory
3. Check browser DevTools Network tab for 404 errors

### Sound Not Playing

1. Verify `shutter.mp3` exists and is valid
2. Check browser console for autoplay restrictions
3. Sound only plays on user interaction (click)

### Slow Loading

1. Optimize image file sizes
2. Use appropriate dimensions (don't serve 6000px images)
3. Consider lazy loading for below-fold images

---

## Related

- [Customization](customization.md) - Styling your gallery
- [Quick Start](../getting-started/quick-start.md) - Getting started
