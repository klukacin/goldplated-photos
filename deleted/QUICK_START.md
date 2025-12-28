# Quick Start Guide

## Your photo gallery is ready!

The development server is running at: **http://localhost:4321**

## Current Structure

You have example albums set up in `src/content/albums/`:

```
2025/
├── index.md (Collection page)
├── Birthdays/
│   ├── index.md (Category page)
│   ├── Johns-Birthday/
│   │   └── index.md (Album - ready for photos)
│   └── Marias-Birthday/
│       └── index.md (Password-protected album)
└── Vacations/
    └── Summer-Trip/ (Not yet configured)
```

## Add Your First Photos

### Option 1: Add photos to an existing album

```bash
# Copy your photos to Johns-Birthday album
cp /path/to/your/photos/*.jpg src/content/albums/2025/Birthdays/Johns-Birthday/

# Restart the dev server to see them
```

### Option 2: Create a new album

```bash
# Create new album folder
mkdir -p src/content/albums/2025/My-Album

# Create the metadata file
cat > src/content/albums/2025/My-Album/index.md << 'EOF'
---
title: "My First Album"
description: "A collection of my favorite photos"
date: 2025-12-21
style: "grid"
sort: "date-desc"
tags: ["family", "friends"]
---

This is my first photo album!
EOF

# Add your photos
cp /path/to/your/photos/*.jpg src/content/albums/2025/My-Album/
```

## Testing Password Protection

The "Marias-Birthday" album is password protected. The password is: **maria2025**

Navigate to `/2025/Birthdays/Marias-Birthday` to test it.

## Features to Try

1. **Browse nested folders** - Click through 2025 → Birthdays → Johns-Birthday
2. **View EXIF data** - Click "Info" on any photo (works with photos that have EXIF data)
3. **Download photos** - Click "Download" under any photo
4. **Download albums** - Click "Download Album" button to get all photos as ZIP
5. **Lightbox viewer** - Click any photo to view full-screen
6. **Password protection** - Try accessing the password-protected album
7. **Responsive design** - Resize your browser or test on mobile

## Different View Styles

Edit an album's `index.md` to change the `style` field:

- `grid` - Uniform grid layout (default)
- `masonry` - Pinterest-style masonry layout
- `slideshow` - Full-width slideshow

## Next Steps

1. **Add your photo collection** - Copy your organized photo folders to `src/content/albums/`
2. **Add metadata files** - Create `index.md` in each folder with title, description, etc.
3. **Customize styling** - Edit colors in `src/layouts/Layout.astro`
4. **Build for production** - Run `npm run build` when ready to deploy

## Troubleshooting

**Don't see your photos?**
- Make sure the photo files are in a supported format (`.jpg`, `.jpeg`, `.png`, `.gif`, `.webp`)
- Check that each album folder has an `index.md` file
- Restart the dev server after adding new albums

**Build errors?**
- Check that all `index.md` files have valid YAML frontmatter
- Ensure required fields (title) are present in all metadata files

## Support

Check the main README.md for full documentation and deployment instructions.

Enjoy your photo gallery!
