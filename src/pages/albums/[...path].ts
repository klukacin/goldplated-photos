import type { APIRoute } from 'astro';
import fs from 'fs/promises';
import path from 'path';
import { lookup } from 'mrmime';
import { resolveFileAccess, getAccessCookieValue } from '../../lib/access';

export const prerender = false;

export const GET: APIRoute = async ({ params, url, cookies }) => {
  const photoPath = params.path;

  if (!photoPath) {
    return new Response('Path required', { status: 400 });
  }

  // Security: Prevent directory traversal
  if (photoPath.includes('..') || photoPath.startsWith('/') || photoPath.includes('\0')) {
    return new Response('Invalid path', { status: 400 });
  }

  // Security: Block metadata — markdown (contains passwords), .meta cache,
  // and any dotfile/dot-directory segment
  const segments = photoPath.split('/');
  if (photoPath.endsWith('.md') || segments.some(s => s.startsWith('.'))) {
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
    return new Response(fileBuffer, {
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
