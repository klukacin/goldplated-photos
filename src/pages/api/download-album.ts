import type { APIRoute } from 'astro';
import JSZip from 'jszip';
import fs from 'fs/promises';
import path from 'path';
import { getAlbumByPath } from '../../lib/albums';
import { resolveAlbumAccess, getAccessCookieValue } from '../../lib/access';

export const prerender = false;

export const POST: APIRoute = async ({ request, cookies }) => {
  try {
    const { albumPath } = await request.json();

    if (!albumPath) {
      return new Response(JSON.stringify({ error: 'Album path is required' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // SECURITY: Block path traversal attempts
    if (albumPath.includes('..') || albumPath.startsWith('/') || albumPath.includes('\0')) {
      return new Response(JSON.stringify({ error: 'Invalid path' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    const album = await getAlbumByPath(albumPath);
    if (!album) {
      return new Response(JSON.stringify({ error: 'Album not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Downloads must be explicitly enabled for the album
    if (!album.data.allowDownload) {
      return new Response(JSON.stringify({ error: 'Downloads are not enabled for this album' }), {
        status: 403,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // SECURITY: Full access check — album AND ancestors (cookie or share token)
    const access = await resolveAlbumAccess(
      albumPath,
      getAccessCookieValue(cookies),
      request.headers.get('X-Album-Token')
    );
    if (!access.hasAccess) {
      return new Response(JSON.stringify({ error: 'Unauthorized - album is protected' }), {
        status: 401,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    const albumDir = path.join(process.cwd(), 'src/content/albums', albumPath);

    try {
      await fs.access(albumDir);
    } catch {
      return new Response(JSON.stringify({ error: 'Album directory not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Read all photo files
    const files = await fs.readdir(albumDir);
    const photoExtensions = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif'];
    const photoFiles = files.filter(file => {
      const ext = path.extname(file).toLowerCase();
      return photoExtensions.includes(ext);
    });

    if (photoFiles.length === 0) {
      return new Response(JSON.stringify({ error: 'No photos in this album' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Create ZIP file
    const zip = new JSZip();

    for (const filename of photoFiles) {
      const filePath = path.join(albumDir, filename);
      const fileData = await fs.readFile(filePath);
      zip.file(filename, fileData);
    }

    const zipBuffer = await zip.generateAsync({
      type: 'nodebuffer',
      compression: 'DEFLATE',
      compressionOptions: { level: 6 }
    });

    const albumName = albumPath.split('/').pop() || 'album';

    return new Response(new Uint8Array(zipBuffer), {
      status: 200,
      headers: {
        'Content-Type': 'application/zip',
        'Content-Disposition': `attachment; filename="${albumName}.zip"`,
        'Content-Length': zipBuffer.length.toString()
      }
    });
  } catch (error) {
    console.error('Error creating album ZIP:', error);
    return new Response(JSON.stringify({ error: 'Failed to create album download' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' }
    });
  }
};
