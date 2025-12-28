import type { APIRoute } from 'astro';
import sharp from 'sharp';
import fs from 'fs/promises';
import path from 'path';
import { createHash } from 'crypto';

export const prerender = false;

// Thumbnail sizes
const THUMBNAIL_SIZES = {
  small: 400,   // Grid thumbnails
  medium: 1200, // Lightbox preview
  large: 1920   // Full view
};

export const GET: APIRoute = async ({ request }) => {
  const url = new URL(request.url);
  const photoPath = url.searchParams.get('path');
  const size = url.searchParams.get('size') || 'small';

  if (!photoPath) {
    return new Response('Path required', { status: 400 });
  }

  // Security: Prevent directory traversal
  if (photoPath.includes('..') || photoPath.startsWith('/')) {
    return new Response('Invalid path', { status: 400 });
  }

  // Validate size
  if (!['small', 'medium', 'large'].includes(size)) {
    return new Response('Invalid size', { status: 400 });
  }

  const width = THUMBNAIL_SIZES[size as keyof typeof THUMBNAIL_SIZES];
  const sourcePath = path.join(process.cwd(), 'src/content/albums', photoPath);

  // Extract album directory and filename
  const albumDir = path.dirname(photoPath);
  const filename = path.basename(photoPath);
  const ext = path.extname(filename);
  const nameWithoutExt = path.basename(filename, ext);

  // Cache in album's .meta directory: src/content/albums/{album}/.meta/thumbnails/{size}/{filename}
  const cacheDir = path.join(process.cwd(), 'src/content/albums', albumDir, '.meta/thumbnails', size);
  const cachePath = path.join(cacheDir, filename);

  try {
    // Check if thumbnail already exists in cache
    try {
      await fs.access(cachePath);
      console.log(`[Thumbnail] Cache hit: ${photoPath} (${size})`);

      const cachedBuffer = await fs.readFile(cachePath);
      return new Response(cachedBuffer, {
        status: 200,
        headers: {
          'Content-Type': 'image/jpeg',
          'Content-Length': cachedBuffer.length.toString(),
          'Cache-Control': 'public, max-age=31536000, immutable',
        },
      });
    } catch {
      // Cache miss, continue to generate
      console.log(`[Thumbnail] Cache miss: ${photoPath} (${size})`);
    }

    // Check if source file exists
    await fs.access(sourcePath);

    // Ensure cache directory exists
    await fs.mkdir(cacheDir, { recursive: true });

    // Generate thumbnail with Sharp
    const thumbnail = await sharp(sourcePath)
      .rotate() // Auto-rotate based on EXIF orientation
      .resize(width, null, {
        width: width,
        withoutEnlargement: true,
        fit: 'inside'
      })
      .jpeg({
        quality: 85,
        progressive: true
      })
      .toBuffer();

    // Save to cache
    await fs.writeFile(cachePath, thumbnail);
    console.log(`[Thumbnail] Generated and cached: ${photoPath} (${size})`);

    // Return thumbnail
    return new Response(thumbnail, {
      status: 200,
      headers: {
        'Content-Type': 'image/jpeg',
        'Content-Length': thumbnail.length.toString(),
        'Cache-Control': 'public, max-age=31536000, immutable',
      },
    });
  } catch (error) {
    console.error('[Thumbnail] Error generating thumbnail:', error);

    // Fallback to original image if thumbnail generation fails
    try {
      const original = await fs.readFile(sourcePath);
      return new Response(original, {
        status: 200,
        headers: {
          'Content-Type': 'image/jpeg',
          'Content-Length': original.length.toString(),
          'Cache-Control': 'public, max-age=31536000, immutable',
        },
      });
    } catch {
      return new Response('Thumbnail generation failed', { status: 500 });
    }
  }
};
