import type { APIRoute } from 'astro';
import { timingSafeEqual } from 'crypto';
import { getAlbumByPath, getAncestors, getAllDescendants } from '../../lib/albums';

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
    const { albumPath, password, unlockedTokens, providedToken } = await request.json();

    if (!albumPath) {
      return new Response(JSON.stringify({ success: false, error: 'Missing album path' }), {
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

    const ancestors = await getAncestors(albumPath);
    const unlocked = new Set<string>(unlockedTokens || []);

    // Handle token-based access
    if (providedToken === album.data.token && album.data.allowAnonymous) {
      unlocked.add(album.data.token);
      return new Response(JSON.stringify({
        success: true,
        unlockedTokens: [...unlocked],
        shareableUrl: `/${albumPath}?token=${album.data.token}`
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Handle password validation
    if (password) {
      const correctPassword = album.data.password;

      if (!correctPassword) {
        // Album is not password protected
        return new Response(JSON.stringify({
          success: true,
          unlockedTokens: [...unlocked]
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' }
        });
      }

      if (safeCompare(password, correctPassword)) {
        // Password correct - unlock this album
        unlocked.add(album.data.token);

        // CASCADE: Unlock all descendants without passwords
        const descendants = await getAllDescendants(albumPath);
        descendants.forEach(desc => {
          if (!desc.data.password) {
            unlocked.add(desc.data.token);
          }
        });

        return new Response(JSON.stringify({
          success: true,
          unlockedTokens: [...unlocked],
          shareableUrl: `/${albumPath}?token=${album.data.token}`
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' }
        });
      } else {
        return new Response(JSON.stringify({
          success: false,
          error: 'Incorrect password'
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' }
        });
      }
    }

    // Check access without password (for verification)
    // Check if album itself requires password
    if (album.data.password && !unlocked.has(album.data.token)) {
      return new Response(JSON.stringify({
        success: false,
        requiresPassword: true,
        blockingAlbum: {
          path: albumPath,
          title: album.data.title
        }
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' }
      });
    }

    // Check if any ancestor requires password
    for (const ancestor of ancestors) {
      const ancestorPath = ancestor.id.replace('/index.md', '');
      if (ancestor.data.password && !unlocked.has(ancestor.data.token)) {
        return new Response(JSON.stringify({
          success: false,
          requiresPassword: true,
          blockingAlbum: {
            path: ancestorPath,
            title: ancestor.data.title
          }
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' }
        });
      }
    }

    // Access granted
    return new Response(JSON.stringify({
      success: true,
      unlockedTokens: [...unlocked]
    }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' }
    });

  } catch (error) {
    console.error('Error checking access:', error);
    return new Response(JSON.stringify({ success: false, error: 'Server error' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' }
    });
  }
};
