import type { APIRoute } from 'astro';
import fs from 'fs/promises';
import path from 'path';
import { lookup } from 'mrmime';
import { resolveFileAccess, getAccessCookieValue } from '../../lib/access';
import { isSafeMediaPath } from '../../lib/access-core';
import { isDisabledImageFormat } from '../../lib/image-formats';

export const prerender = false;

export const GET: APIRoute = async ({ params, url, cookies }) => {
  const photoPath = params.path;

  if (!photoPath) {
    return new Response('Path required', { status: 400 });
  }

  // Security: shared rule set — traversal, absolute paths, NUL, backslash
  // (a separator on Windows), dot segments (.meta cache) and markdown
  // (contains passwords) are all rejected in one place.
  if (!isSafeMediaPath(photoPath)) {
    return new Response('Not found', { status: 404 });
  }

  // A format the feature flags have disabled (FEATURE_HEIC=0) is invisible:
  // discovery skips it, and this route must not serve it to a remembered URL.
  if (isDisabledImageFormat(photoPath)) {
    return new Response('Not found', { status: 404 });
  }

  // SECURITY: Enforce album access (password / share token) on the file itself
  const access = await resolveFileAccess(
    photoPath,
    getAccessCookieValue(cookies),
    url.searchParams.get('token')
  );
  if (!access.hasAccess) {
    return new Response('Unauthorized', { status: 401 });
  }

  const fullPath = path.join(process.cwd(), 'src/content/albums', photoPath);

  try {
    // Check if file exists
    await fs.access(fullPath);

    // Read the file
    const fileBuffer = await fs.readFile(fullPath);

    // Determine MIME type
    const ext = path.extname(photoPath).toLowerCase();
    const mimeType = lookup(ext) || 'application/octet-stream';

    // Return the image (private: browser may cache, shared caches must not)
    return new Response(new Uint8Array(fileBuffer), {
      status: 200,
      headers: {
        'Content-Type': mimeType,
        'Cache-Control': access.isProtected
          ? 'private, max-age=31536000'
          : 'public, max-age=31536000, immutable',
      },
    });
  } catch (error) {
    console.error('Error serving photo:', error);
    return new Response('Photo not found', { status: 404 });
  }
};
