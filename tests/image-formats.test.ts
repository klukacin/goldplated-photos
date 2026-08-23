/**
 * Sharp reads HEIC/HEIF and browsers mostly do not — every rule in
 * src/lib/image-formats.ts exists because of that gap. Safari renders HEIC,
 * so a gallery full of iPhone originals looks perfect on the photographer's
 * Mac and is a wall of broken images for everybody on Chrome or Firefox.
 * These tests pin down which files have to be transcoded before a browser
 * ever sees them.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  IMAGE_EXTENSIONS,
  BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS,
  imageExtension,
  isImageFilename,
  isBrowserDisplayableImage,
  needsBrowserTranscode,
  browserSafeImageUrl,
  chooseThumbnailFormat,
} from '../src/lib/image-formats';

describe('imageExtension', () => {
  it('returns the lowercased extension with its dot', () => {
    expect(imageExtension('photo.JPG')).toBe('.jpg');
    expect(imageExtension('IMG_4021.HEIC')).toBe('.heic');
    expect(imageExtension('shot.webp')).toBe('.webp');
  });

  it('takes the last extension of a multi-dot name', () => {
    expect(imageExtension('2025-06-14.wedding.ana.jpeg')).toBe('.jpeg');
  });

  it('returns empty string when there is no extension', () => {
    expect(imageExtension('README')).toBe('');
    expect(imageExtension('')).toBe('');
  });

  it('ignores a query string and hash, so a cache-busted URL still classifies', () => {
    // PhotoGrid hands these in as `/albums/…/photo.heic?v=3`
    expect(imageExtension('/albums/2025/x/photo.heic?v=3')).toBe('.heic');
    expect(imageExtension('/albums/2025/x/photo.jpg#top')).toBe('.jpg');
  });

  it('does not mistake a dot in a directory name for an extension', () => {
    expect(imageExtension('/albums/16.album-name/photo')).toBe('');
  });
});

describe('isImageFilename', () => {
  it('accepts every format the gallery ingests, HEIC and WebP included', () => {
    for (const ext of ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif']) {
      expect(isImageFilename(`photo${ext}`), ext).toBe(true);
      expect(isImageFilename(`photo${ext.toUpperCase()}`), ext).toBe(true);
    }
  });

  it('rejects videos and everything else', () => {
    expect(isImageFilename('clip.mp4')).toBe(false);
    expect(isImageFilename('index.md')).toBe(false);
    expect(isImageFilename('archive.zip')).toBe(false);
  });
});

describe('isBrowserDisplayableImage', () => {
  it('says yes to the formats every browser decodes', () => {
    expect(isBrowserDisplayableImage('photo.jpg')).toBe(true);
    expect(isBrowserDisplayableImage('photo.jpeg')).toBe(true);
    expect(isBrowserDisplayableImage('photo.png')).toBe(true);
    expect(isBrowserDisplayableImage('photo.gif')).toBe(true);
  });

  it('says yes to WebP — browsers have decoded it for years', () => {
    expect(isBrowserDisplayableImage('photo.webp')).toBe(true);
    expect(isBrowserDisplayableImage('photo.WEBP')).toBe(true);
  });

  it('says no to HEIC and HEIF regardless of case', () => {
    expect(isBrowserDisplayableImage('IMG_4021.HEIC')).toBe(false);
    expect(isBrowserDisplayableImage('img.heic')).toBe(false);
    expect(isBrowserDisplayableImage('img.heif')).toBe(false);
  });

  it('says no to a file that is not an image at all', () => {
    expect(isBrowserDisplayableImage('clip.mov')).toBe(false);
    expect(isBrowserDisplayableImage('notes')).toBe(false);
  });

  it('is exactly the ingest list minus HEIC/HEIF', () => {
    const gap = IMAGE_EXTENSIONS.filter(e => !BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS.includes(e));
    expect(gap).toEqual(['.heic', '.heif']);
  });
});

describe('needsBrowserTranscode', () => {
  it('is true only for images the gallery accepts but a browser cannot paint', () => {
    expect(needsBrowserTranscode('IMG_4021.HEIC')).toBe(true);
    expect(needsBrowserTranscode('scan.heif')).toBe(true);
  });

  it('is false for images a browser paints directly', () => {
    expect(needsBrowserTranscode('photo.jpg')).toBe(false);
    expect(needsBrowserTranscode('photo.webp')).toBe(false);
  });

  it('is false for non-images — a video is not the thumbnail endpoint\'s problem', () => {
    expect(needsBrowserTranscode('clip.mp4')).toBe(false);
  });
});

describe('browserSafeImageUrl', () => {
  const thumb = '/api/thumbnail?path=a%2Fb.heic&size=large';

  it('hands back the original when the browser can display it', () => {
    expect(browserSafeImageUrl('/albums/a/b.jpg', '/api/thumbnail?path=a%2Fb.jpg&size=large'))
      .toBe('/albums/a/b.jpg');
  });

  it('substitutes the transcoded URL for a HEIC original', () => {
    expect(browserSafeImageUrl('/albums/a/b.heic', thumb)).toBe(thumb);
    expect(browserSafeImageUrl('/albums/a/B.HEIC', thumb)).toBe(thumb);
  });

  it('looks past a cache-busting query on the original', () => {
    expect(browserSafeImageUrl('/albums/a/b.heic?v=3', thumb)).toBe(thumb);
  });
});

describe('the admin panel\'s hand-copied lists', () => {
  // admin/server.js is plain Node ESM and admin/js/utils.js is a classic
  // <script>; neither can import the TypeScript module, so both keep a copy.
  // A copy that drifts is a copy that lies — the admin would start accepting a
  // hero slide the home page cannot render, and nothing else would notice.
  const PROJECT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

  function copiedList(relPath: string, name: string): string[] {
    const source = fs.readFileSync(path.join(PROJECT_ROOT, relPath), 'utf-8');
    const match = source.match(new RegExp(`const ${name} = (\\[[^\\]]*\\]);`));
    if (!match) throw new Error(`${name} not found in ${relPath}`);
    return JSON.parse(match[1].replace(/'/g, '"'));
  }

  it('admin/server.js agrees on which formats a browser can paint', () => {
    expect(copiedList('admin/server.js', 'BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS'))
      .toEqual(BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS);
  });

  it('admin/server.js agrees on which formats an album accepts', () => {
    expect(copiedList('admin/server.js', 'IMAGE_EXTENSIONS')).toEqual(IMAGE_EXTENSIONS);
  });

  it('admin/js/utils.js agrees on which formats a browser can paint', () => {
    expect(copiedList('admin/js/utils.js', 'BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS'))
      .toEqual(BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS);
  });
});

describe('chooseThumbnailFormat', () => {
  it('picks WebP when the client advertises it', () => {
    expect(chooseThumbnailFormat('image/avif,image/webp,image/apng,*/*;q=0.8')).toBe('webp');
  });

  it('falls back to JPEG for a wildcard Accept — social crawlers send exactly this', () => {
    expect(chooseThumbnailFormat('*/*')).toBe('jpeg');
  });

  it('falls back to JPEG when there is no Accept header at all', () => {
    expect(chooseThumbnailFormat(null)).toBe('jpeg');
    expect(chooseThumbnailFormat('')).toBe('jpeg');
  });

  it('never negotiates HEIC, even to a client that claims to want it', () => {
    // Safari sends image/heic in some contexts. Passing the original through
    // would defeat the whole point of the endpoint: it exists to produce
    // something every browser can paint.
    expect(chooseThumbnailFormat('image/heic,image/heif,*/*')).toBe('jpeg');
    expect(chooseThumbnailFormat('image/heic,image/webp,*/*')).toBe('webp');
  });
});
