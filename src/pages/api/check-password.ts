import type { APIRoute } from 'astro';
import { timingSafeEqual } from 'crypto';
import { getAlbumByPath } from '../../lib/albums';

// SECURITY: Timing-safe string comparison to prevent timing attacks
function safeCompare(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }
  return timingSafeEqual(Buffer.from(a), Buffer.from(b));
}

export const prerender = false;

export const POST: APIRoute = async ({ request }) => {
  try {
    const { albumPath, password } = await request.json();

    if (!albumPath || !password) {
      return new Response(JSON.stringify({ success: false, error: 'Missing parameters' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    const album = await getAlbumByPath(albumPath);

    if (!album) {
      return new Response(JSON.stringify({ success: false, error: 'Album not found' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    const correctPassword = album.data.password;

    if (!correctPassword) {
      // Album is not password protected
      return new Response(JSON.stringify({ success: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    const isCorrect = safeCompare(password, correctPassword);

    return new Response(JSON.stringify({ success: isCorrect }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    });
  } catch (error) {
    console.error('Error checking password:', error);
    return new Response(JSON.stringify({ success: false, error: 'Server error' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' }
    });
  }
};
