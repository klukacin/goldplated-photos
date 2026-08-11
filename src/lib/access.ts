/**
 * Unified access control for albums and media files (Astro glue layer).
 *
 * This module is the ONLY place that decides whether a request may see an
 * album's content. It is used by the album page (SSR), the unlock endpoint
 * and every media route (originals, thumbnails, EXIF, video info, watermark,
 * album download). The pure logic lives in ./access-core (unit-tested).
 */
import type { AstroCookies } from 'astro';
import { getAlbumByPath, getAncestors, type Album } from './albums';
import {
  ACCESS_COOKIE,
  ACCESS_COOKIE_MAX_AGE,
  resolveChainAccess,
  serializeAccessCookie,
  type AccessResult
} from './access-core';

export * from './access-core';

/** Write the signed access cookie with the standard flags. */
export function setAccessCookie(cookies: AstroCookies, tokens: string[]): void {
  cookies.set(ACCESS_COOKIE, serializeAccessCookie(tokens), {
    httpOnly: true,
    secure: import.meta.env.PROD,
    sameSite: 'strict',
    maxAge: ACCESS_COOKIE_MAX_AGE,
    path: '/'
  });
}

/** Read the raw access cookie value from an Astro cookies jar. */
export function getAccessCookieValue(cookies: AstroCookies): string | undefined {
  return cookies.get(ACCESS_COOKIE)?.value;
}

/**
 * Resolve access for an album path.
 *
 * @param albumPath   album id, e.g. "2025/weddings/john-jane"
 * @param cookieValue raw value of the `album-access` cookie (or undefined)
 * @param providedToken share token from `?token=` or `X-Album-Token` (or null)
 */
export async function resolveAlbumAccess(
  albumPath: string,
  cookieValue: string | undefined | null,
  providedToken?: string | null
): Promise<AccessResult> {
  const album = await getAlbumByPath(albumPath);
  const ancestors = await getAncestors(albumPath);
  // Bottom-up chain: the album itself (when it exists), then its ancestors.
  const chain: Album[] = album ? [album, ...ancestors] : [...ancestors];
  return resolveChainAccess(chain, cookieValue, providedToken);
}

/**
 * Resolve access for a media file inside the albums tree. Walks up from the
 * file's directory to the nearest existing album and applies the same rules
 * as the album page. Files outside any album are treated as public.
 */
export async function resolveFileAccess(
  filePath: string,
  cookieValue: string | undefined | null,
  providedToken?: string | null
): Promise<AccessResult> {
  const dir = filePath.split('/').slice(0, -1).join('/');
  if (!dir) {
    // File at the root of the albums tree — nothing to lock it with.
    return resolveChainAccess([], cookieValue, providedToken);
  }
  return resolveAlbumAccess(dir, cookieValue, providedToken);
}
