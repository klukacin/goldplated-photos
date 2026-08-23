/**
 * The grid's sort order, extracted from the album page's client script into
 * pure logic. The rules worth pinning: photos without an EXIF date go to the
 * end in both EXIF directions (undated is unknown, not ancient), 'custom'
 * restores the server's render order, and an unknown option changes nothing.
 */
import { describe, it, expect } from 'vitest';
import {
  comparePhotoEntries,
  sortOptionForAlbumSort,
  type SortablePhotoEntry,
} from '../src/lib/photo-sort';

function entry(over: Partial<SortablePhotoEntry> = {}): SortablePhotoEntry {
  return { filename: 'a.jpg', index: 0, mtimeMs: 0, exifDateMs: 0, size: 0, ...over };
}

function sorted(sortBy: string, entries: SortablePhotoEntry[]): string[] {
  return [...entries]
    .sort((a, b) => comparePhotoEntries(sortBy, a, b))
    .map(e => e.filename);
}

describe('sortOptionForAlbumSort', () => {
  it('maps every schema sort value to its dropdown option', () => {
    expect(sortOptionForAlbumSort('date-desc')).toBe('date-newest');
    expect(sortOptionForAlbumSort('date-asc')).toBe('date-oldest');
    expect(sortOptionForAlbumSort('exif-desc')).toBe('exif-newest');
    expect(sortOptionForAlbumSort('exif-asc')).toBe('exif-oldest');
    expect(sortOptionForAlbumSort('name')).toBe('name-asc');
    expect(sortOptionForAlbumSort('custom')).toBe('custom');
  });

  it('falls back to date-oldest for anything unknown', () => {
    expect(sortOptionForAlbumSort('surprise')).toBe('date-oldest');
    expect(sortOptionForAlbumSort('')).toBe('date-oldest');
  });
});

describe('comparePhotoEntries', () => {
  const photos = [
    entry({ filename: 'c.jpg', index: 0, mtimeMs: 300, exifDateMs: 100, size: 5 }),
    entry({ filename: 'a.jpg', index: 1, mtimeMs: 100, exifDateMs: 300, size: 15 }),
    entry({ filename: 'b.jpg', index: 2, mtimeMs: 200, exifDateMs: 0, size: 10 }),
  ];

  it('custom restores the server render order', () => {
    expect(sorted('custom', photos)).toEqual(['c.jpg', 'a.jpg', 'b.jpg']);
  });

  it('sorts by name in both directions', () => {
    expect(sorted('name-asc', photos)).toEqual(['a.jpg', 'b.jpg', 'c.jpg']);
    expect(sorted('name-desc', photos)).toEqual(['c.jpg', 'b.jpg', 'a.jpg']);
  });

  it('sorts by mtime in both directions', () => {
    expect(sorted('date-oldest', photos)).toEqual(['a.jpg', 'b.jpg', 'c.jpg']);
    expect(sorted('date-newest', photos)).toEqual(['c.jpg', 'b.jpg', 'a.jpg']);
  });

  it('sorts by size in both directions', () => {
    expect(sorted('size-smallest', photos)).toEqual(['c.jpg', 'b.jpg', 'a.jpg']);
    expect(sorted('size-largest', photos)).toEqual(['a.jpg', 'b.jpg', 'c.jpg']);
  });

  it('puts photos without an EXIF date at the end, in both EXIF directions', () => {
    expect(sorted('exif-oldest', photos)).toEqual(['c.jpg', 'a.jpg', 'b.jpg']);
    expect(sorted('exif-newest', photos)).toEqual(['a.jpg', 'c.jpg', 'b.jpg']);
  });

  it('treats two undated photos as equal rather than reordering them', () => {
    const undatedA = entry({ filename: 'x.jpg' });
    const undatedB = entry({ filename: 'y.jpg' });
    expect(comparePhotoEntries('exif-oldest', undatedA, undatedB)).toBe(0);
  });

  it('leaves the order alone for an unknown sort option', () => {
    expect(comparePhotoEntries('nonsense', photos[0], photos[1])).toBe(0);
  });
});
