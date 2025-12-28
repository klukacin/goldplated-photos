# Configuration

All configuration options for Goldplated Photos.

---

## Album Configuration

Each album has an `index.md` file with YAML frontmatter:

```yaml
---
title: "Album Title"           # Required: Display name
thumbnail: "cover.jpg"         # Optional: Cover photo filename
password: "secret"             # Optional: Password protection
order: 1                       # Optional: Sort order (lower = first)
hidden: true                   # Optional: Hide from listings
isCollection: true             # Optional: Contains sub-albums, not photos
allowAnonymous: true           # Optional: Allow token-based access
style: "masonry"               # Optional: Default view mode
---
```

### Field Reference

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `title` | string | *required* | Album display name |
| `thumbnail` | string | first photo | Cover photo filename |
| `password` | string | none | Password for access |
| `order` | number | none | Sort order (lower first) |
| `hidden` | boolean | `false` | Hide from album listings |
| `isCollection` | boolean | `false` | Album contains sub-albums |
| `allowAnonymous` | boolean | `false` | Allow URL token access |
| `style` | string | `"grid"` | Default view: grid, masonry, single |

---

## Thumbnail Configuration

Thumbnails are generated on-demand by `src/pages/api/thumbnail.ts`:

```typescript
const THUMBNAIL_SIZES = {
  small: 400,   // Grid thumbnails
  medium: 1200, // Lightbox preview
  large: 1920   // Lightbox full
};
```

### Modify Thumbnail Quality

Edit the Sharp options in `thumbnail.ts`:

```typescript
.jpeg({ quality: 85, progressive: true })
```

### Clear Thumbnail Cache

```bash
rm -rf .meta/thumbnails
```

Thumbnails regenerate automatically on next request.

---

## Site Configuration

### Astro Config (astro.config.mjs)

```javascript
export default defineConfig({
  output: 'server',  // Required for SSR password protection
  adapter: node({
    mode: 'standalone'
  }),
  site: 'https://your-domain.com'
});
```

### Content Collection Schema

Album schema defined in `src/content/config.ts`:

```typescript
const albumsCollection = defineCollection({
  type: 'content',
  schema: z.object({
    title: z.string(),
    password: z.string().optional(),
    thumbnail: z.string().optional(),
    order: z.number().optional(),
    hidden: z.boolean().default(false),
    isCollection: z.boolean().default(false),
    allowAnonymous: z.boolean().default(false),
    style: z.enum(['grid', 'masonry', 'single']).default('grid'),
    token: z.string().default(() => crypto.randomUUID())
  })
});
```

---

## Rate Limiting

Password attempts are rate-limited in `src/lib/rate-limit.ts`:

```typescript
const MAX_ATTEMPTS = 10;           // Attempts before lockout
const LOCKOUT_DURATION = 15 * 60 * 1000;  // 15 minutes
```

---

## Design System

Typography and colors are configured in `src/layouts/Layout.astro`:

### Fonts

```css
--font-heading: 'Syne', sans-serif;
--font-body: 'DM Sans', sans-serif;
--font-signature: 'Satisfy', cursive;
```

### Colors

```css
--color-text: #0a0a0a;
--color-text-secondary: #404040;
--color-text-muted: #666666;
--color-focus: #2563eb;
```

### Spacing Scale

```css
--space-xs: 0.5rem;
--space-sm: 1rem;
--space-md: 1.5rem;
--space-lg: 2rem;
--space-xl: 3rem;
--space-2xl: 5rem;
--space-3xl: 8rem;
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `4321` | Server port |
| `HOST` | `localhost` | Server host |

---

## File Naming Conventions

!!! warning "Avoid Dots in Folder Names"
    Astro normalizes folder names, which can break paths.

    - `16.album-name` becomes `16album-name`
    - Use dashes instead: `16-album-name`

### Recommended Structure

```
src/content/albums/
└── 2025/
    └── 01-january/
        └── new-years-party/
            ├── index.md
            └── *.jpg
```

---

## Next Steps

- [Adding Albums](../guides/adding-albums.md) - Album creation guide
- [Deployment](../guides/deployment.md) - Production deployment
