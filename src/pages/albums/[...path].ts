import type { APIRoute } from 'astro';
import fs from 'fs/promises';
import path from 'path';
import { lookup } from 'mrmime';

export const prerender = false;

export const GET: APIRoute = async ({ params }) => {
  const photoPath = params.path;

  if (!photoPath) {
    return new Response('Path required', { status: 400 });
  }

  // Security: Prevent directory traversal
  if (photoPath.includes('..') || photoPath.startsWith('/')) {
    return new Response('Invalid path', { status: 400 });
  }

  // Security: Block access to index.md files (contain passwords)
  if (photoPath.endsWith('index.md') || photoPath.includes('/index.md')) {
    return new Response('Not found', { status: 404 });
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

    // Return the image
    return new Response(fileBuffer, {
      status: 200,
      headers: {
        'Content-Type': mimeType,
        'Cache-Control': 'public, max-age=31536000, immutable',
      },
    });
  } catch (error) {
    console.error('Error serving photo:', error);
    return new Response('Photo not found', { status: 404 });
  }
};
