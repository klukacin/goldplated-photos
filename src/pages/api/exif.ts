import type { APIRoute } from 'astro';
import * as exifr from 'exifr';
import fs from 'fs/promises';
import path from 'path';

export const prerender = false;

export const POST: APIRoute = async ({ request }) => {
  try {
    const { photoUrl } = await request.json();

    if (!photoUrl) {
      return new Response(JSON.stringify({ error: 'Photo URL is required' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Convert URL to file path
    const photoPath = photoUrl.replace('/albums/', '');

    // SECURITY: Block path traversal attempts
    if (photoPath.includes('..') || photoPath.startsWith('/')) {
      return new Response(JSON.stringify({ error: 'Invalid path' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    const fullPath = path.join(process.cwd(), 'src/content/albums', photoPath);

    try {
      await fs.access(fullPath);
    } catch {
      return new Response(JSON.stringify({ error: 'Photo not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Extract EXIF data
    try {
      const exifData = await exifr.parse(fullPath, {
        tiff: true,
        exif: true,
        gps: true,
        iptc: true,
        ifd0: true,
        xmp: true,  // For Rating
        ifd1: false,
        interop: false,
      });

      return new Response(JSON.stringify({ exif: exifData || {} }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' }
      });
    } catch (parseError) {
      console.error('[EXIF] Parse error for:', photoPath, parseError);
      return new Response(JSON.stringify({ exif: {} }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' }
      });
    }
  } catch (error) {
    console.error('Error extracting EXIF data:', error);
    return new Response(JSON.stringify({ error: 'Failed to extract EXIF data' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' }
    });
  }
};
