/**
 * Which image formats a browser can actually paint — and which have to be
 * converted before one ever sees them.
 *
 * Sharp here reads HEIC/HEIF (libvips is built with libheif), so the gallery
 * happily ingests what an iPhone shoots. Browsers are the problem: **Chrome
 * and Firefox cannot decode HEIC at all**, and neither can the Facebook or X
 * crawlers. Safari can — which is exactly how this ships unnoticed from a Mac:
 * the photographer sees the album, most of the audience sees broken images.
 *
 * So every HEIC URL that reaches an `<img src>`, a CSS background or an
 * `og:image` must go through `/api/thumbnail`, which always writes JPEG or
 * WebP. Downloads are exempt — a file is a file, and the client asking for
 * the original wants the original.
 *
 * Pure and dependency-free: the album loader, the thumbnail endpoint and the
 * PhotoGrid client bundle all import it, and `tests/image-formats.test.ts`
 * covers it.
 *
 * The lists themselves live in src/site-features.mjs — the shared module the
 * admin server also reads — so the FEATURE_HEIC flag moves gallery discovery
 * and admin upload filters together, and neither keeps a copy.
 */
import {
  IMAGE_EXTENSIONS,
  BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS,
  HEIC_EXTENSIONS,
} from '../site-features.mjs';

/**
 * Every image format the gallery discovers in an album directory. With
 * FEATURE_HEIC=0 this list has no `.heic`/`.heif` in it.
 *
 * BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS is the subset every browser decodes.
 * WebP belongs there — Chrome, Firefox, Safari and Edge have all shipped it
 * for years, and the thumbnail endpoint already serves it by content
 * negotiation. No flag can widen it.
 */
export { IMAGE_EXTENSIONS, BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS };

/**
 * Lowercased extension (with the dot) of a filename, path or URL.
 * Query strings and hashes are stripped first, because callers pass URLs
 * that carry the thumbnail cache-buster (`…/photo.heic?v=3`).
 */
export function imageExtension(filenameOrUrl: string): string {
  if (!filenameOrUrl) return '';
  const withoutQuery = filenameOrUrl.split(/[?#]/)[0];
  const base = withoutQuery.slice(withoutQuery.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  if (dot <= 0) return '';
  return base.slice(dot).toLowerCase();
}

/** Is this a file the gallery treats as a photo? */
export function isImageFilename(filenameOrUrl: string): boolean {
  return IMAGE_EXTENSIONS.includes(imageExtension(filenameOrUrl));
}

/** Can a browser paint this file as-is? */
export function isBrowserDisplayableImage(filenameOrUrl: string): boolean {
  return BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS.includes(imageExtension(filenameOrUrl));
}

/**
 * True for a photo the gallery accepts but a browser cannot decode — today
 * that is HEIC and HEIF. Anything answering true must be served through
 * `/api/thumbnail` rather than as the original file.
 */
export function needsBrowserTranscode(filenameOrUrl: string): boolean {
  return isImageFilename(filenameOrUrl) && !isBrowserDisplayableImage(filenameOrUrl);
}

/**
 * Pick the URL to hand an `<img>`: the original when the browser can render
 * it, the transcoded one when it cannot. Call sites that want full quality
 * (the lightbox's Original toggle, the admin preview) use this so a HEIC
 * silently downgrades to a 1920px JPEG instead of showing a broken image.
 */
export function browserSafeImageUrl(originalUrl: string, transcodedUrl: string): string {
  return needsBrowserTranscode(originalUrl) ? transcodedUrl : originalUrl;
}

/**
 * True for a file whose format the site knows of but the current feature set
 * has switched off — today that means HEIC/HEIF under FEATURE_HEIC=0. Media
 * routes (originals, thumbnails) answer 404 for these instead of serving a
 * file the rest of the site pretends does not exist.
 */
export function isDisabledImageFormat(filenameOrUrl: string): boolean {
  const ext = imageExtension(filenameOrUrl);
  return HEIC_EXTENSIONS.includes(ext) && !IMAGE_EXTENSIONS.includes(ext);
}

/**
 * Thumbnail content negotiation. Only ever WebP or JPEG — the endpoint's
 * whole purpose is producing something universal, so the source format never
 * gets a say and HEIC is never passed through, whatever the client asks for.
 */
export function chooseThumbnailFormat(accept: string | null | undefined): 'webp' | 'jpeg' {
  return (accept || '').includes('image/webp') ? 'webp' : 'jpeg';
}
