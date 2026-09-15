/**
 * Response headers every server-rendered page and API route carries.
 *
 * Prerendered pages (`/photos`, `/home`, tag pages) are served as static
 * files and do not pass through here — `Layout.astro` carries a referrer
 * meta tag for those, and `public/.htaccess` sets the same headers at the
 * edge for a host that honours it. No CSP: PhotoGrid and the album page use
 * inline scripts throughout, and a nonce rollout is its own change.
 */
import { defineMiddleware } from 'astro:middleware';

export const onRequest = defineMiddleware(async (_context, next) => {
  const response = await next();
  const headers = response.headers;
  // The password form must not be framed by another site.
  headers.set('X-Frame-Options', 'DENY');
  // A file served with the wrong type is not to be sniffed into a script.
  headers.set('X-Content-Type-Options', 'nosniff');
  // A share token rides in the album URL; cross-origin requests (Google Fonts,
  // any link in body.md) get the origin only.
  headers.set('Referrer-Policy', 'strict-origin-when-cross-origin');
  return response;
});
