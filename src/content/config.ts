import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';

const albums = defineCollection({
  loader: glob({ pattern: '**/index.md', base: './src/content/albums' }),
  schema: z.object({
    title: z.string(),
    description: z.string().optional(),
    date: z.coerce.date().optional(),
    token: z.string(), // Internal album id used in the (signed) access cookie — grants nothing by itself
    password: z.string().optional(),
    // Random secret for link-sharing (generate via admin or scripts/add-share-token.mjs).
    // Presenting it via ?token= unlocks the album without a password. An album with a
    // shareToken and no password is reachable ONLY via the secret link.
    shareToken: z.string().optional(),
    // DEPRECATED: no longer consulted. Access via token requires an explicit shareToken.
    allowAnonymous: z.boolean().default(true),
    sort: z.enum(['date-asc', 'date-desc', 'exif-asc', 'exif-desc', 'name', 'custom']).default('date-desc'),
    style: z.enum(['grid', 'masonry', 'slideshow', 'single-column']).default('single-column'),
    thumbnail: z.string().optional(),
    tags: z.array(z.string()).optional(),
    isCollection: z.boolean().default(false), // true if this is a folder containing albums
    order: z.number().optional(), // Lower numbers appear first in album listings
    hidden: z.boolean().default(false), // Hide from album listings (accessible via direct link)
    allowDownload: z.boolean().default(false), // Enable Download Album button
  }),
});

const home = defineCollection({
  type: 'content',
  schema: z.object({
    type: z.enum(['intro', 'card']).optional(),
    title: z.string().optional(),
    image: z.string().optional(),
    imagePosition: z.enum(['left', 'right']).optional(),
    link: z.string().optional(),
    order: z.number().optional(),
  }),
});

export const collections = {
  albums,
  home,
};
