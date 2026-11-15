import type { APIRoute } from 'astro';
import sharp from 'sharp';
import fs from 'fs/promises';
import path from 'path';
import { siteConfig } from '../../config';
import { resolveFileAccess, getAccessCookieValue } from '../../lib/access';
import { isSafeMediaPath } from '../../lib/access-core';

export const prerender = false;

/**
 * Watermark API - Generates images with watermark for social sharing
 *
 * GET /api/watermark?path=album/photo.jpg
 * Returns: Watermarked JPEG image for download
 */
export const GET: APIRoute = async ({ request, cookies }) => {
  // FEATURE_WATERMARK off ⇒ the route does not exist. The Instagram share
  // button that calls it is hidden under the same flag in PhotoGrid.astro.
  if (!siteConfig.features.watermark) {
    return new Response('Not found', { status: 404 });
  }
  const url = new URL(request.url);
  const photoPath = url.searchParams.get('path');

  if (!photoPath) {
    return new Response('Path required', { status: 400 });
  }

  // Security: shared rule set — traversal, absolute paths, NUL, backslash
  // (a separator on Windows), dot segments (.meta cache) and markdown
  // (contains passwords) are all rejected in one place.
  if (!isSafeMediaPath(photoPath)) {
    return new Response('Not found', { status: 404 });
  }

  // SECURITY: Enforce album access
  const access = await resolveFileAccess(
    photoPath,
    getAccessCookieValue(cookies),
    url.searchParams.get('token')
  );
  if (!access.hasAccess) {
    return new Response('Unauthorized', { status: 401 });
  }

  const sourcePath = path.join(process.cwd(), 'src/content/albums', photoPath);

  try {
    // Check if source file exists
    await fs.access(sourcePath);

    // Load image and get metadata
    const image = sharp(sourcePath);
    const metadata = await image.metadata();

    if (!metadata.width || !metadata.height) {
      return new Response('Could not read image dimensions', { status: 500 });
    }

    // Get watermark settings from config
    const watermarkText = siteConfig.watermark.text;
    const opacity = siteConfig.watermark.opacity;
    const position = siteConfig.watermark.position;

    // Calculate font size based on image width (responsive)
    const fontSize = Math.max(24, Math.floor(metadata.width / 40));
    const padding = Math.max(20, Math.floor(metadata.width / 60));

    // Calculate text position based on config
    let textAnchor: string;
    let xPosition: number;

    switch (position) {
      case 'bottom-left':
        textAnchor = 'start';
        xPosition = padding;
        break;
      case 'bottom-center':
        textAnchor = 'middle';
        xPosition = metadata.width / 2;
        break;
      case 'bottom-right':
      default:
        textAnchor = 'end';
        xPosition = metadata.width - padding;
        break;
    }

    const yPosition = metadata.height - padding;

    // Create watermark SVG with text shadow for visibility on any background
    const svgText = `
      <svg width="${metadata.width}" height="${metadata.height}" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
            <feDropShadow dx="1" dy="1" stdDeviation="2" flood-color="black" flood-opacity="0.5"/>
          </filter>
        </defs>
        <text
          x="${xPosition}"
          y="${yPosition}"
          text-anchor="${textAnchor}"
          font-family="Arial, Helvetica, sans-serif"
          font-size="${fontSize}px"
          font-weight="500"
          fill="rgba(255, 255, 255, ${opacity})"
          filter="url(#shadow)"
        >${watermarkText}</text>
      </svg>
    `;

    // Composite watermark onto image
    const watermarked = await image
      .rotate() // Auto-rotate based on EXIF
      .composite([{
        input: Buffer.from(svgText),
        gravity: 'southeast'
      }])
      .jpeg({ quality: 90, progressive: true })
      .toBuffer();

    // Generate download filename
    const originalFilename = path.basename(photoPath);
    const nameWithoutExt = originalFilename.replace(/\.[^.]+$/, '');
    const downloadFilename = `${nameWithoutExt}-share.jpg`;

    return new Response(new Uint8Array(watermarked), {
      status: 200,
      headers: {
        'Content-Type': 'image/jpeg',
        'Content-Length': watermarked.length.toString(),
        'Content-Disposition': `attachment; filename="${downloadFilename}"`,
        'Cache-Control': 'no-cache', // Don't cache watermarked images
      },
    });
  } catch (error) {
    console.error('[Watermark] Error generating watermarked image:', error);
    return new Response('Failed to generate watermarked image', { status: 500 });
  }
};
