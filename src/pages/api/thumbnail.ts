import type { APIRoute } from 'astro';
import sharp from 'sharp';
import fs from 'fs/promises';
import path from 'path';
import { lookup } from 'mrmime';
import { resolveFileAccess, getAccessCookieValue } from '../../lib/access';
import { isSafeMediaPath } from '../../lib/access-core';
import { imageJobSemaphore } from '../../lib/semaphore';
import { chooseThumbnailFormat, needsBrowserTranscode, isDisabledImageFormat } from '../../lib/image-formats';

export const prerender = false;

// Thumbnail sizes
const THUMBNAIL_SIZES = {
  small: 400,   // Grid thumbnails
  medium: 1200, // Lightbox preview
  large: 1920   // Full view
};

const FORMATS = {
  webp: { contentType: 'image/webp', ext: 'webp' },
  jpeg: { contentType: 'image/jpeg', ext: 'jpg' }
} as const;

export const GET: APIRoute = async ({ request, cookies }) => {
  const url = new URL(request.url);
  const photoPath = url.searchParams.get('path');
  const size = url.searchParams.get('size') || 'small';

  if (!photoPath) {
    return new Response('Path required', { status: 400 });
  }

  // Security: shared rule set — traversal, absolute paths, NUL, backslash
  // (a separator on Windows), dot segments (.meta cache) and markdown
  // (contains passwords — the original-file fallback below could otherwise
  // serve index.md verbatim) are all rejected in one place.
  if (!isSafeMediaPath(photoPath)) {
    return new Response('Not found', { status: 404 });
  }

  // A format the feature flags have disabled (FEATURE_HEIC=0) is invisible —
  // no thumbnails for files the rest of the site pretends do not exist.
  if (isDisabledImageFormat(photoPath)) {
    return new Response('Not found', { status: 404 });
  }

  // Validate size
  if (!['small', 'medium', 'large'].includes(size)) {
    return new Response('Invalid size', { status: 400 });
  }

  // SECURITY: Enforce album access (password / share token)
  const access = await resolveFileAccess(
    photoPath,
    getAccessCookieValue(cookies),
    url.searchParams.get('token')
  );
  if (!access.hasAccess) {
    return new Response('Unauthorized', { status: 401 });
  }
  const cacheControl = access.isProtected
    ? 'private, max-age=31536000'
    : 'public, max-age=31536000, immutable';

  // Content negotiation: serve WebP to clients that accept it (smaller files),
  // JPEG otherwise. The source format never gets a vote — Sharp decodes HEIC
  // but Chrome and Firefox do not, so this endpoint always *writes* one of the
  // two universal formats. The disk cache is keyed by format; responses carry
  // `Vary: Accept` so shared caches keep the variants apart.
  const formatName = chooseThumbnailFormat(request.headers.get('accept'));
  const wantsWebp = formatName === 'webp';
  const format = FORMATS[formatName];

  const width = THUMBNAIL_SIZES[size as keyof typeof THUMBNAIL_SIZES];
  const sourcePath = path.join(process.cwd(), 'src/content/albums', photoPath);

  // Extract album directory and filename
  const albumDir = path.dirname(photoPath);
  const filename = path.basename(photoPath);

  // Cache in album's .meta directory:
  // src/content/albums/{album}/.meta/thumbnails/{size}/{filename}.{fmt}
  const cacheDir = path.join(process.cwd(), 'src/content/albums', albumDir, '.meta/thumbnails', size);
  const cachePath = path.join(cacheDir, `${filename}.${format.ext}`);

  const baseHeaders = {
    'Content-Type': format.contentType,
    'Cache-Control': cacheControl,
    'Vary': 'Accept',
  };

  try {
    const sourceStat = await fs.stat(sourcePath);

    // Serve from cache only when it is newer than the source file — replacing
    // a photo under the same name invalidates its thumbnails automatically.
    try {
      const cacheStat = await fs.stat(cachePath);
      if (cacheStat.mtimeMs >= sourceStat.mtimeMs) {
        const cachedBuffer = await fs.readFile(cachePath);
        return new Response(new Uint8Array(cachedBuffer), {
          status: 200,
          headers: {
            ...baseHeaders,
            'Content-Length': cachedBuffer.length.toString(),
          },
        });
      }
    } catch {
      // Cache miss, continue to generate
    }

    // Ensure cache directory exists
    await fs.mkdir(cacheDir, { recursive: true });

    // Generate thumbnail with Sharp (bounded concurrency — a cold large album
    // must not saturate CPU/memory with parallel libvips jobs)
    const thumbnail = await imageJobSemaphore.run(() => {
      const pipeline = sharp(sourcePath)
        .rotate() // Auto-rotate based on EXIF orientation
        .resize(width, null, {
          withoutEnlargement: true,
          fit: 'inside'
        });
      return wantsWebp
        ? pipeline.webp({ quality: 82 }).toBuffer()
        : pipeline.jpeg({ quality: 85, progressive: true }).toBuffer();
    });

    // Save to cache
    await fs.writeFile(cachePath, thumbnail);

    // Return thumbnail
    return new Response(new Uint8Array(thumbnail), {
      status: 200,
      headers: {
        ...baseHeaders,
        'Content-Length': thumbnail.length.toString(),
      },
    });
  } catch (error) {
    console.error('[Thumbnail] Error generating thumbnail:', error);

    // Fallback to original image if thumbnail generation fails.
    // Use the real MIME type and a SHORT cache so a transient Sharp failure
    // is not cached for a year by browsers/CDNs.
    //
    // Only for formats the browser can paint, though. Handing Chrome the raw
    // bytes of a HEIC is not a fallback — it is the same broken image with a
    // 200 status and an `image/heic` label, which also poisons any cache that
    // trusted the header. Better to fail loudly so the log names the photo.
    if (needsBrowserTranscode(photoPath)) {
      return new Response('Thumbnail generation failed', { status: 500 });
    }

    try {
      const original = await fs.readFile(sourcePath);
      const mimeType = lookup(path.extname(sourcePath).toLowerCase()) || 'application/octet-stream';
      return new Response(new Uint8Array(original), {
        status: 200,
        headers: {
          'Content-Type': mimeType,
          'Content-Length': original.length.toString(),
          'Cache-Control': 'no-cache',
        },
      });
    } catch {
      return new Response('Thumbnail generation failed', { status: 500 });
    }
  }
};
