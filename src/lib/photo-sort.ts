/**
 * The photo sort order, as pure comparison logic.
 *
 * The album page's client script sorts the rendered grid by reordering DOM
 * nodes; what order they end up in is decided here, on plain records read
 * from the nodes' data attributes. Keeping the comparator out of the DOM
 * code is what makes rules like "photos without an EXIF date sort to the
 * end" testable — see tests/photo-sort.test.ts.
 */

/** One grid item's sortable facts, read from its data-* attributes. */
export interface SortablePhotoEntry {
  filename: string;
  /**
   * The server's render position. `sort: custom` albums arrive with
   * `photoOrder` already applied server-side, so "custom" simply restores
   * this index.
   */
  index: number;
  /** File mtime in ms since epoch; 0 when unknown. */
  mtimeMs: number;
  /** EXIF capture date in ms since epoch; 0 when the photo has none. */
  exifDateMs: number;
  /** File size in bytes. */
  size: number;
}

/**
 * Map an album's frontmatter `sort` value to the sort dropdown's option.
 * Unknown values fall back to date-oldest, as the page always has.
 */
export function sortOptionForAlbumSort(albumSort: string): string {
  const mapping: Record<string, string> = {
    'date-desc': 'date-newest',
    'date-asc': 'date-oldest',
    'exif-desc': 'exif-newest',
    'exif-asc': 'exif-oldest',
    'name': 'name-asc',
    'custom': 'custom'
  };
  return mapping[albumSort] || 'date-oldest';
}

/**
 * Compare two photos under a sort option. Unknown options compare equal, so
 * the grid keeps whatever order it had.
 */
export function comparePhotoEntries(
  sortBy: string,
  a: SortablePhotoEntry,
  b: SortablePhotoEntry
): number {
  switch (sortBy) {
    case 'custom':
      return a.index - b.index;

    case 'name-asc':
      return a.filename.localeCompare(b.filename);

    case 'name-desc':
      return b.filename.localeCompare(a.filename);

    case 'date-newest':
    case 'date-oldest':
      return sortBy === 'date-newest' ? b.mtimeMs - a.mtimeMs : a.mtimeMs - b.mtimeMs;

    case 'exif-newest':
    case 'exif-oldest': {
      // Photos without an EXIF date (0) sort to the end in either direction —
      // an undated photo is not "older than everything", it is unknown.
      if (a.exifDateMs === 0 && b.exifDateMs === 0) return 0;
      if (a.exifDateMs === 0) return 1;
      if (b.exifDateMs === 0) return -1;
      return sortBy === 'exif-newest'
        ? b.exifDateMs - a.exifDateMs
        : a.exifDateMs - b.exifDateMs;
    }

    case 'size-largest':
    case 'size-smallest':
      return sortBy === 'size-largest' ? b.size - a.size : a.size - b.size;

    default:
      return 0;
  }
}
