/**
 * Builds the URL a visitor shares for one photo (or the whole album).
 *
 * Pure so the query-string interplay is testable: `?photo=` for a deep link,
 * plus the album's share token when the album is protected — where the token
 * is the *first* parameter if no photo is named, and the second if one is.
 * Getting that separator wrong quietly produces a link that opens the
 * password form instead of the photo, which no test caught while this lived
 * inside PhotoGrid's client script.
 */

export interface ShareUrlConfig {
  /** Origin + any base, e.g. "https://example.com" (no trailing slash). */
  siteUrl: string;
  /** Album path under /photos, e.g. "2025/weddings/ana-ivan". */
  albumPath: string;
  /** Whether the album (or an ancestor) is password/token protected. */
  isProtected: boolean;
  /** The album's shareToken; empty when it has none. */
  albumToken: string;
}

export function buildPhotoShareUrl(config: ShareUrlConfig, filename: string): string {
  let url = `${config.siteUrl}/photos/${config.albumPath}`;

  if (filename) {
    url += `?photo=${encodeURIComponent(filename)}`;
  }

  // Add token for protected albums so the link works without the password
  if (config.isProtected && config.albumToken) {
    url += (filename ? '&' : '?') + `token=${config.albumToken}`;
  }

  return url;
}
