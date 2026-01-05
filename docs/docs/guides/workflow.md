# Author Workflow

This guide describes the recommended workflow for managing your photo galleries with Goldplated Photos. The approach keeps your photos safe on local storage while making them easy to publish online.

## Overview

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│  1. Organize │───▶│ 2. Generate │───▶│  3. Refine  │───▶│  4. Deploy  │
│    Albums    │    │   Metadata  │    │   Settings  │    │   Online    │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
     Local              npm run           npm run           npm run
    Storage             update            admin             deploy
```

## Step 1: Organize Albums Locally

Keep all your photos on local storage (external drive recommended) as your **master copy**. This ensures:

- **Backup safety** - Your photos are never only on a server
- **Easy organization** - Use your file manager to arrange albums
- **Version control** - Git tracks your album structure, not the photos

### Create Album Structure

Organize photos into folders under `src/content/albums/`:

```
src/content/albums/
├── 2024/
│   ├── summer-vacation/
│   │   ├── beach-day/
│   │   │   └── *.jpg
│   │   └── hiking/
│   │       └── *.jpg
│   └── wedding-smith/
│       └── *.jpg
└── portraits/
    └── family/
        └── *.jpg
```

!!! tip "Folder Naming"
    - Use lowercase with dashes: `summer-vacation` not `Summer Vacation`
    - Avoid dots after numbers: `16-album` not `16.album`
    - Keep names URL-friendly

## Step 2: Generate Metadata

Run the update command to auto-generate `index.md` files for new albums:

```bash
npm run update
```

This command:

- Scans for folders without `index.md`
- Creates default metadata files
- Sets album titles from folder names
- Preserves existing `index.md` files

### Generated index.md

```yaml
---
title: "Summer Vacation"
description: ""
thumbnail: ""
password: ""
order: 0
hidden: false
isCollection: false
---
```

## Step 3: Refine with Admin Panel

Start the admin panel to fine-tune your albums:

```bash
npm run admin
```

Open http://localhost:4444 in your browser.

### Admin Panel Features

| Feature | Description |
|---------|-------------|
| **Edit Metadata** | Change titles, descriptions, and settings |
| **Set Thumbnails** | Choose cover photos for albums |
| **Add Passwords** | Protect sensitive albums |
| **Preview** | See how albums will look |
| **Manage Structure** | Create collections and sub-albums |

### Common Tasks

#### Password Protection

```yaml
---
title: "Client Wedding"
password: "clientsecret123"
---
```

#### Set Custom Thumbnail

```yaml
---
title: "Beach Photos"
thumbnail: "sunset-best.jpg"
---
```

#### Create a Collection

Set `isCollection: true` to make a folder display sub-albums instead of photos:

```yaml
---
title: "2024"
isCollection: true
---
```

#### Hide from Listings

```yaml
---
title: "Private Album"
hidden: true
---
```

Album is still accessible via direct URL but won't appear in listings.

## Step 4: Deploy Online

When you're satisfied with your changes, deploy to your server:

```bash
npm run deploy
```

This command:

1. Builds the production site
2. Syncs files to your server via rsync
3. Restarts the server process
4. Albums with `--delete` flag removes old files

!!! warning "Deploy Syncs Everything"
    The deploy command syncs your local albums to the server. Albums deleted locally will be removed from the server too.

### Verify Deployment

After deploying, check your live site to ensure:

- [ ] New albums appear correctly
- [ ] Thumbnails load properly
- [ ] Password protection works
- [ ] All photos are accessible

## Complete Workflow Example

Here's a typical workflow for adding a new album:

```bash
# 1. Copy photos to the albums folder
cp -r ~/Photos/Wedding-2024 src/content/albums/2024/wedding-johnson/

# 2. Generate initial metadata
npm run update

# 3. Start admin panel to configure
npm run admin
# → Set title: "Johnson Wedding"
# → Add password: "johnson2024"
# → Choose thumbnail
# → Save changes

# 4. Preview locally (optional)
npm run dev
# → Check http://localhost:4321/photos/2024/wedding-johnson

# 5. Deploy to production
npm run deploy
```

## Tips & Best Practices

### Storage Strategy

| Location | Purpose |
|----------|---------|
| External Drive | Master copy of all photos |
| Project Folder | Working copy synced from master |
| Server | Published galleries |

### Regular Maintenance

- **Weekly**: Back up your external drive
- **Monthly**: Review and clean up old albums
- **After changes**: Always deploy to keep server in sync

### Working with Large Collections

For very large photo libraries:

1. Keep only active albums in the project folder
2. Archive completed albums to cold storage
3. Use collections to organize by year/category

## Troubleshooting

### Album not appearing after deploy

1. Check `hidden: false` in `index.md`
2. Verify folder structure is correct
3. Run `npm run update` to regenerate metadata
4. Clear browser cache

### Thumbnails not generating

1. Delete `.meta/thumbnails` folder
2. Redeploy - thumbnails regenerate on demand

### Password not working

1. Check password in `index.md` (no quotes needed for simple passwords)
2. Clear browser cookies
3. Wait 15 minutes if rate-limited

## Related Guides

- [Adding Albums](adding-albums.md) - Detailed album creation
- [Admin Panel](admin-panel.md) - Full admin panel reference
- [Deployment](deployment.md) - Server configuration
