/**
 * src/site-features.mjs is the single source of truth for feature flags and
 * format lists — src/config.ts, src/lib/image-formats.ts and admin/server.js
 * all read it. These tests pin the resolution rules: every flag defaults ON
 * (current behaviour), the usual falsy spellings turn one OFF, and turning
 * `heic` off shrinks the ingest list that everything downstream consumes.
 */
import { describe, it, expect } from 'vitest';
import {
  resolveFeatures,
  imageExtensionsFor,
  BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS,
  HEIC_EXTENSIONS,
  VIDEO_EXTENSIONS,
  features,
  IMAGE_EXTENSIONS,
} from '../src/site-features.mjs';

const ROUTE_FLAGS = ['heic', 'watermark', 'search', 'tags', 'proofing'] as const;
const ENV_NAMES: Record<(typeof ROUTE_FLAGS)[number], string> = {
  heic: 'FEATURE_HEIC',
  watermark: 'FEATURE_WATERMARK',
  search: 'FEATURE_SEARCH',
  tags: 'FEATURE_TAGS',
  proofing: 'FEATURE_PROOFING',
};

describe('resolveFeatures', () => {
  it('defaults every feature flag ON with an empty environment', () => {
    const f = resolveFeatures({});
    for (const name of ROUTE_FLAGS) {
      expect(f[name], name).toBe(true);
    }
    expect(f.enablePhotoSharing).toBe(true);
    // The one exception: video thumbnails were never implemented, so the
    // pre-flag behaviour to preserve is OFF.
    expect(f.enableVideoThumbnails).toBe(false);
    expect(f.slideshowIntervalMs).toBe(5000);
  });

  it.each(['0', 'false', 'off', 'no', 'FALSE', ' Off '])(
    'turns a flag off for %j',
    (value) => {
      for (const name of ROUTE_FLAGS) {
        expect(resolveFeatures({ [ENV_NAMES[name]]: value })[name], name).toBe(false);
      }
    }
  );

  it.each(['1', 'true', 'on', 'yes', ''])('leaves a flag on for %j', (value) => {
    for (const name of ROUTE_FLAGS) {
      expect(resolveFeatures({ [ENV_NAMES[name]]: value })[name], name).toBe(true);
    }
  });

  it('flags are independent — turning one off leaves the others on', () => {
    const f = resolveFeatures({ FEATURE_PROOFING: '0' });
    expect(f.proofing).toBe(false);
    expect(f.search).toBe(true);
    expect(f.tags).toBe(true);
    expect(f.watermark).toBe(true);
    expect(f.heic).toBe(true);
  });

  it('reads the slideshow interval, ignoring garbage and non-positive values', () => {
    expect(resolveFeatures({ SLIDESHOW_INTERVAL_MS: '8000' }).slideshowIntervalMs).toBe(8000);
    expect(resolveFeatures({ SLIDESHOW_INTERVAL_MS: 'soon' }).slideshowIntervalMs).toBe(5000);
    expect(resolveFeatures({ SLIDESHOW_INTERVAL_MS: '-1' }).slideshowIntervalMs).toBe(5000);
  });
});

describe('imageExtensionsFor', () => {
  it('includes HEIC/HEIF when the flag is on (the default)', () => {
    expect(imageExtensionsFor(resolveFeatures({}))).toEqual([
      '.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif',
    ]);
  });

  it('shrinks to browser-displayable formats when heic is off', () => {
    const list = imageExtensionsFor(resolveFeatures({ FEATURE_HEIC: '0' }));
    expect(list).toEqual(BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS);
    for (const ext of HEIC_EXTENSIONS) {
      expect(list).not.toContain(ext);
    }
  });

  it('never lets a flag put HEIC into the browser-displayable list', () => {
    // That list is what raw-served public assets are held to; no flag may
    // widen it, because nothing in the public/ pipeline converts formats.
    for (const ext of HEIC_EXTENSIONS) {
      expect(BROWSER_DISPLAYABLE_IMAGE_EXTENSIONS).not.toContain(ext);
    }
  });
});

describe('the resolved singletons', () => {
  it('match a fresh resolution of this process environment', () => {
    expect(features).toEqual(resolveFeatures(process.env as Record<string, string>));
    expect(IMAGE_EXTENSIONS).toEqual(imageExtensionsFor(features));
  });

  it('exports the video list the gallery and admin both filter by', () => {
    expect(VIDEO_EXTENSIONS).toEqual(['.mp4', '.webm', '.mov', '.avi', '.mkv', '.m4v']);
  });
});
