/**
 * Centralized Site Configuration
 *
 * This file contains all site-wide settings including:
 * - Site identity and metadata
 * - Social sharing defaults
 * - Author information
 * - Feature flags
 *
 * Feature flags live in src/site-features.mjs (plain ESM, so the admin
 * server can read the same file) and are re-exported here typed. They are
 * resolved from environment variables at server start; see that module for
 * the FEATURE_* names and the prerender caveat.
 */
import { features } from './site-features.mjs';

/** The resolved feature set — shape inferred from the shared module. */
export type SiteFeatures = typeof features;

export interface SiteConfig {
  // Site Identity
  name: string;
  description: string;
  url: string;

  // Social/SEO Defaults
  defaultOgImage: string;
  twitterHandle: string;
  twitterCreator: string;

  // Author Info
  author: {
    name: string;
    email?: string;
  };

  // Watermark Settings
  watermark: {
    text: string;        // Text to display (defaults to site URL hostname)
    opacity: number;     // 0-1 (0.5 = 50%)
    position: 'bottom-right' | 'bottom-left' | 'bottom-center';
  };

  // Feature Flags
  features: SiteFeatures;
}

export const siteConfig: SiteConfig = {
  // Site Identity
  name: "Kristijan Lukačin Photography",
  description: "Photography by Kristijan Lukačin - Welcome to my digital archives",
  url: import.meta.env.SITE_URL || "https://kristijan.lukacin.com",

  // Social/SEO Defaults
  defaultOgImage: "/images/og-default.jpg",
  twitterHandle: "",  // Twitter/X handle without @ (leave empty if none)
  twitterCreator: "Kristijan Lukačin",

  // Author Info
  author: {
    name: "Kristijan Lukačin",
    email: "photo@kristijan.lukacin.com"
  },

  // Watermark Settings (for Instagram sharing)
  watermark: {
    text: "kristijan.lukacin.com",  // Text shown on watermarked images
    opacity: 0.5,                    // 50% opacity (subtle)
    position: 'bottom-right'         // Bottom-right corner
  },

  // Feature Flags — resolved once at server start from FEATURE_* env vars
  features
};

/**
 * Convert a relative path to an absolute URL
 * @param path - Relative path (e.g., "/images/photo.jpg")
 * @returns Full URL (e.g., "https://example.com/images/photo.jpg")
 */
export function absoluteUrl(path: string): string {
  const base = siteConfig.url.endsWith('/')
    ? siteConfig.url.slice(0, -1)
    : siteConfig.url;
  const cleanPath = path.startsWith('/') ? path : `/${path}`;
  return `${base}${cleanPath}`;
}

/**
 * Get the OG image URL for an album
 * @param albumPath - Album path (e.g., "2024/Events/Wedding")
 * @param coverPhoto - Cover photo filename or null
 * @returns Absolute URL to the thumbnail
 */
export function getAlbumOgImageUrl(albumPath: string, coverPhoto: string | null): string {
  if (!coverPhoto) {
    return absoluteUrl(siteConfig.defaultOgImage);
  }

  const thumbnailPath = `/api/thumbnail?path=${encodeURIComponent(`${albumPath}/${coverPhoto}`)}&size=large`;
  return absoluteUrl(thumbnailPath);
}

/**
 * Get the OG image URL for a specific photo
 * @param albumPath - Album path (e.g., "2024/Events/Wedding")
 * @param photoFilename - Photo filename (e.g., "DSC_1234.jpg")
 * @returns Absolute URL to the large thumbnail
 */
export function getPhotoOgImageUrl(albumPath: string, photoFilename: string): string {
  const thumbnailPath = `/api/thumbnail?path=${encodeURIComponent(`${albumPath}/${photoFilename}`)}&size=large`;
  return absoluteUrl(thumbnailPath);
}
