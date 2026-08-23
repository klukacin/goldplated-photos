import type { APIRoute } from 'astro';
import * as exifr from 'exifr';
import fs from 'fs/promises';
import path from 'path';
import { resolveFileAccess, getAccessCookieValue } from '../../lib/access';
import { isSafeMediaPath } from '../../lib/access-core';

export const prerender = false;

export const POST: APIRoute = async ({ request, cookies }) => {
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

    // SECURITY: shared rule set — traversal, absolute paths, NUL, backslash
    // (a separator on Windows), dot segments (.meta cache) and markdown
    // (contains passwords) are all rejected in one place.
    if (!isSafeMediaPath(photoPath)) {
      return new Response(JSON.stringify({ error: 'Photo not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // SECURITY: Enforce album access (EXIF can contain GPS coordinates).
    // Same token sources as the other media routes: ?token= or X-Album-Token.
    const url = new URL(request.url);
    const access = await resolveFileAccess(
      photoPath,
      getAccessCookieValue(cookies),
      url.searchParams.get('token') || request.headers.get('X-Album-Token')
    );
    if (!access.hasAccess) {
      return new Response(JSON.stringify({ error: 'Unauthorized' }), {
        status: 401,
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
      // Cast: exifr's TS Options type is missing several documented keys (ifd0 etc.)
      const exifData = await exifr.parse(fullPath, {
        tiff: true,
        exif: true,
        gps: true,
        iptc: true,
        ifd0: true,
        xmp: true,  // For Rating
        ifd1: false,
        interop: false,
      } as unknown as Parameters<typeof exifr.parse>[1]);

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
