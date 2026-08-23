/**
 * The one source of truth for feature flags and media format lists.
 *
 * Plain ESM on purpose, so every runtime in the project can load the same
 * file instead of keeping a hand-copy:
 *
 *   - src/config.ts re-exports `features` into `siteConfig.features` (typed
 *     by inference — allowJs is on and the JSDoc below carries the shapes)
 *   - src/lib/image-formats.ts derives its extension lists from it
 *   - admin/server.js imports it directly (Node ESM) and re-resolves the
 *     flags after loading .env
 *   - admin/js/* are classic <script>s that cannot import anything, so the
 *     admin server hands them the computed lists through GET /api/config,
 *     the same channel that already carries previewUrl
 *
 * Flags come from environment variables and default ON (except where noted),
 * so an unconfigured install behaves exactly as before the flags existed.
 * Set FEATURE_SEARCH=0 (or false/off/no) in the environment or .env and the
 * feature is gone at the next server start — no rebuild for SSR routes.
 * Prerendered output is the exception: the tag pages and the /photos search
 * box are static HTML baked by `astro build`, so for those a flag change
 * takes effect on the next build, not the next restart.
 */

/** Values that switch a flag off; anything else (or unset) leaves it on. */
const FALSY = new Set(['0', 'false', 'off', 'no']);

/**
 * @param {Record<string, string | undefined>} env
 * @param {string} name
 * @param {boolean} defaultOn
 */
function flag(env, name, defaultOn = true) {
  const raw = env[name];
  if (raw === undefined || raw === '') return defaultOn;
  return !FALSY.has(String(raw).trim().toLowerCase());
}

/**
 * @param {string | undefined} raw
 * @param {number} fallback
 */
function positiveIntOr(raw, fallback) {
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

/**
 * Resolve the feature set from an environment. Pure — the unit tests feed it
 * environments directly; everything else uses the `features` singleton below.
 *
 * @param {Record<string, string | undefined>} env
 */
export function resolveFeatures(env = {}) {
  return {
    /** Accept and serve HEIC/HEIF files (discovery, uploads, media routes). */
    heic: flag(env, 'FEATURE_HEIC'),
    /** The /api/watermark route and the Instagram share button that uses it. */
    watermark: flag(env, 'FEATURE_WATERMARK'),
    /** The /photos/search page and the search box on /photos. */
    search: flag(env, 'FEATURE_SEARCH'),
    /** Tag pages under /photos/tags/* and the tag pills on album pages. */
    tags: flag(env, 'FEATURE_TAGS'),
    /** Kill-switch over per-album `proofing: true` — API and UI both. */
    proofing: flag(env, 'FEATURE_PROOFING'),
    /** ?photo= deep links for individual photo sharing. */
    enablePhotoSharing: flag(env, 'FEATURE_PHOTO_SHARING'),
    /** Off until ffmpeg-based video thumbnails exist on the server. */
    enableVideoThumbnails: flag(env, 'FEATURE_VIDEO_THUMBNAILS', false),
    /** Auto-advance interval for `style: slideshow` albums. */
    slideshowIntervalMs: positiveIntOr(env.SLIDESHOW_INTERVAL_MS, 5000),
  };
}

/** @typedef {ReturnType<typeof resolveFeatures>} Features */

/**
 * The subset of image formats every browser decodes. Chrome and Firefox
 * cannot paint HEIC at all — only Safari can — so this list is what public
 * assets (served raw) are held to, and it does not move with any flag.
 */
export const BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS = ['.jpg', '.jpeg', '.png', '.gif', '.webp'];

/** The formats the `heic` flag governs — accepted by Sharp, not by browsers. */
export const HEIC_EXTENSIONS = ['.heic', '.heif'];

/** Video formats the gallery discovers and the admin accepts for upload. */
export const VIDEO_EXTENSIONS = ['.mp4', '.webm', '.mov', '.avi', '.mkv', '.m4v'];

/**
 * Every image format the gallery ingests under a given feature set. With
 * `heic` off the list shrinks, and everything downstream — album discovery,
 * admin upload filters, media routes — shrinks with it.
 *
 * @param {Features} features
 */
export function imageExtensionsFor(features) {
  return features.heic
    ? [...BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS, ...HEIC_EXTENSIONS]
    : [...BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS];
}

/**
 * The resolved feature set for this process, read once at module load —
 * i.e. at server start. In the browser bundle `process` does not exist and
 * every flag takes its default; that is correct, because a disabled feature
 * never renders the markup the client code would look for.
 */
export const features = resolveFeatures(
  typeof process !== 'undefined' && process.env ? process.env : {}
);

/** The ingest list under this process's feature set. */
export const IMAGE_EXTENSIONS = imageExtensionsFor(features);
