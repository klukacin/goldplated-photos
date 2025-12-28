import type { APIRoute } from 'astro';
import { timingSafeEqual } from 'crypto';
import { getAlbumByPath, getAllDescendants } from '../../lib/albums';
import { isRateLimited, recordFailedAttempt, clearRateLimit, getRemainingAttempts } from '../../lib/rate-limit';

// SECURITY: Timing-safe string comparison
function safeCompare(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }
  return timingSafeEqual(Buffer.from(a), Buffer.from(b));
}

export const prerender = false;

export const POST: APIRoute = async ({ request, cookies, redirect, clientAddress }) => {
  try {
    const formData = await request.formData();
    const albumPath = formData.get('albumPath') as string;
    const password = formData.get('password') as string;
    const returnUrl = formData.get('returnUrl') as string || `/photos/${albumPath}`;

    if (!albumPath || !password) {
      return redirect(`${returnUrl}?error=missing-fields`);
    }

    // Rate limiting
    const ip = clientAddress || request.headers.get('x-forwarded-for') || 'unknown';
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

    // Verify password
    if (!safeCompare(password, correctPassword)) {
      recordFailedAttempt(ip);
      const remaining = getRemainingAttempts(ip);
      return redirect(`${returnUrl}?error=wrong-password&remaining=${remaining}`);
    }

    // Password correct - clear rate limit
    clearRateLimit(ip);

    // Get existing unlocked albums from cookie
    const existingCookie = cookies.get('album-access')?.value;
    let unlocked: string[] = [];
    try {
      unlocked = existingCookie ? JSON.parse(existingCookie) : [];
    } catch {
      unlocked = [];
    }

    // Add this album's token
    if (!unlocked.includes(album.data.token)) {
      unlocked.push(album.data.token);
    }

    // CASCADE: Also unlock all descendants without passwords
    const descendants = await getAllDescendants(albumPath);
    descendants.forEach(desc => {
      if (!desc.data.password && !unlocked.includes(desc.data.token)) {
        unlocked.push(desc.data.token);
      }
    });

    // Set HttpOnly cookie
    cookies.set('album-access', JSON.stringify(unlocked), {
      httpOnly: true,
      secure: import.meta.env.PROD, // HTTPS only in production
      sameSite: 'strict',
      maxAge: 60 * 60 * 24, // 24 hours
      path: '/'
    });

    // Redirect back to album
    return redirect(`/photos/${albumPath}`);

  } catch (error) {
    console.error('Error in unlock:', error);
    return new Response('Server error', { status: 500 });
  }
};
