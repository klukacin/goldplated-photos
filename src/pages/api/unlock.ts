import type { APIRoute } from 'astro';
import { getAlbumByPath } from '../../lib/albums';
import {
  getAccessCookieValue,
  getClientIp,
  parseAccessCookie,
  safeCompare,
  safeReturnUrl,
  setAccessCookie
} from '../../lib/access';
import { isRateLimited, recordFailedAttempt, clearRateLimit, getRemainingAttempts } from '../../lib/rate-limit';

export const prerender = false;

export const POST: APIRoute = async ({ request, cookies, redirect, clientAddress }) => {
  try {
    const formData = await request.formData();
    const albumPath = formData.get('albumPath') as string;
    const password = formData.get('password') as string;
    // Where to go back to comes from the form, and a form can be posted from
    // any page on the internet — so this is reduced to somewhere on this site
    // before it reaches a Location header.
    const returnUrl = safeReturnUrl(formData.get('returnUrl'), `/photos/${albumPath}`);

    if (!albumPath || !password) {
      return redirect(`${returnUrl}?error=missing-fields`);
    }

    // Rate limiting (real client IP, also behind the reverse proxy)
    const ip = getClientIp(clientAddress, request.headers.get('x-forwarded-for'));
    if (isRateLimited(ip)) {
      return redirect(`${returnUrl}?error=rate-limited`);
    }

    const album = await getAlbumByPath(albumPath);
    if (!album) {
      return redirect(`${returnUrl}?error=not-found`);
    }

    const correctPassword = album.data.password;
    if (!correctPassword) {
      // Album is not password protected, redirect to it
      return redirect(`/photos/${albumPath}`);
    }

    // Verify password (timing-safe)
    if (!safeCompare(password, correctPassword)) {
      recordFailedAttempt(ip);
      const remaining = getRemainingAttempts(ip);
      return redirect(`${returnUrl}?error=wrong-password&remaining=${remaining}`);
    }

    // Password correct - clear rate limit
    clearRateLimit(ip);

    // Get existing unlocked albums from the signed cookie (invalid → empty)
    const unlocked = parseAccessCookie(getAccessCookieValue(cookies));

    // Add this album's token. No descendant cascade needed: access resolution
    // grants descendants of an unlocked ancestor automatically, and descendants
    // with their own lock (password or shareToken) must NOT be auto-unlocked.
    if (!unlocked.includes(album.data.token)) {
      unlocked.push(album.data.token);
    }

    // Set signed HttpOnly cookie
    setAccessCookie(cookies, unlocked);

    // Redirect back to album
    return redirect(`/photos/${albumPath}`);

  } catch (error) {
    console.error('Error in unlock:', error);
    return new Response('Server error', { status: 500 });
  }
};
