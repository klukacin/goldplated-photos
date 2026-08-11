import type { APIRoute } from 'astro';
import { ZipArchive } from 'archiver';
import { Readable } from 'node:stream';
import fs from 'fs/promises';
import path from 'path';
import { getAlbumByPath, IMAGE_EXTENSIONS } from '../../lib/albums';
import { resolveAlbumAccess, getAccessCookieValue } from '../../lib/access';

export const prerender = false;

// Total size cap for a ZIP download (sum of source files). Configurable via
// DOWNLOAD_ZIP_MAX_BYTES; defaults to 4 GB.
const MAX_ZIP_BYTES = Number(process.env.DOWNLOAD_ZIP_MAX_BYTES) > 0
  ? Number(process.env.DOWNLOAD_ZIP_MAX_BYTES)
  : 4 * 1024 * 1024 * 1024;

function jsonError(message: string, status: number): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' }
  });
}

export const POST: APIRoute = async ({ request, cookies }) => {
  try {
    const { albumPath } = await request.json();

    if (!albumPath) {
      return jsonError('Album path is required', 400);
    }

    // SECURITY: Block path traversal attempts
    if (albumPath.includes('..') || albumPath.startsWith('/') || albumPath.includes('\0')) {
      return jsonError('Invalid path', 400);
    }

    const album = await getAlbumByPath(albumPath);
    if (!album) {
      return jsonError('Album not found', 404);
    }

    // Downloads must be explicitly enabled for the album
    if (!album.data.allowDownload) {
      return jsonError('Downloads are not enabled for this album', 403);
    }

    // SECURITY: Full access check — album AND ancestors (cookie or share token)
    const access = await resolveAlbumAccess(
      albumPath,
      getAccessCookieValue(cookies),
      request.headers.get('X-Album-Token')
    );
    if (!access.hasAccess) {
      return jsonError('Unauthorized - album is protected', 401);
    }

    const albumDir = path.join(process.cwd(), 'src/content/albums', albumPath);

    try {
      await fs.access(albumDir);
    } catch {
      return jsonError('Album directory not found', 404);
    }

    // Collect photo files with sizes (cap check before any streaming starts)
    const files = await fs.readdir(albumDir);
    const photoFiles = files.filter(file => {
      if (file.startsWith('.')) return false;
      const ext = path.extname(file).toLowerCase();
      return IMAGE_EXTENSIONS.includes(ext);
    });

    if (photoFiles.length === 0) {
      return jsonError('No photos in this album', 404);
    }

    let totalBytes = 0;
    for (const filename of photoFiles) {
      const stat = await fs.stat(path.join(albumDir, filename));
      totalBytes += stat.size;
    }
    if (totalBytes > MAX_ZIP_BYTES) {
      const totalGb = (totalBytes / (1024 ** 3)).toFixed(1);
      const capGb = (MAX_ZIP_BYTES / (1024 ** 3)).toFixed(1);
      return jsonError(`Album is too large to download as ZIP (${totalGb} GB, limit ${capGb} GB)`, 413);
    }

    // Stream the archive — nothing is buffered in memory. JPEGs are already
    // compressed, so 'store' is both faster and effectively as small.
    const archive = new ZipArchive({ store: true });
    for (const filename of photoFiles) {
      archive.file(path.join(albumDir, filename), { name: filename });
    }
    archive.on('error', (err: Error) => {
      console.error('[download-album] Archive error:', err);
      archive.abort();
    });
    archive.finalize();

    const albumName = albumPath.split('/').pop() || 'album';

    return new Response(Readable.toWeb(archive) as ReadableStream, {
      status: 200,
      headers: {
        'Content-Type': 'application/zip',
        'Content-Disposition': `attachment; filename="${albumName}.zip"`,
        'Cache-Control': 'no-store'
      }
    });
  } catch (error) {
    console.error('Error creating album ZIP:', error);
    return jsonError('Failed to create album download', 500);
  }
};
