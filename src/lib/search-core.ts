/**
 * Search matching — pure logic (unit-tested).
 *
 * Album search matches title, description and tags.
 * Photo search matches filename, camera (EXIF make+model) and capture date
 * (both ISO `YYYY-MM-DD` and Croatian `DD.MM.YYYY` spellings).
 */

export interface AlbumSearchable {
  title: string;
  description?: string;
  tags?: string[];
}

export interface PhotoSearchable {
  filename: string;
  camera?: string | null;
  exifDate?: Date | null;
}

/** Case- and whitespace-insensitive normalization */
function norm(value: string): string {
  return value.toLowerCase().trim();
}

export function normalizeQuery(query: string): string {
  return norm(query);
}

export function albumMatchesQuery(query: string, album: AlbumSearchable): boolean {
  const q = norm(query);
  if (!q) return false;
  if (norm(album.title).includes(q)) return true;
  if (album.description && norm(album.description).includes(q)) return true;
  return (album.tags || []).some(tag => norm(tag).includes(q));
}

/** Render the two date spellings a visitor might type */
export function photoDateStrings(exifDate: Date): string[] {
  const year = exifDate.getFullYear();
  const month = (exifDate.getMonth() + 1).toString().padStart(2, '0');
  const day = exifDate.getDate().toString().padStart(2, '0');
  return [
    `${year}-${month}-${day}`, // ISO
    `${day}.${month}.${year}`  // Croatian
  ];
}

export function photoMatchesQuery(query: string, photo: PhotoSearchable): boolean {
  const q = norm(query);
  if (!q) return false;
  if (norm(photo.filename).includes(q)) return true;
  if (photo.camera && norm(photo.camera).includes(q)) return true;
  if (photo.exifDate) {
    if (photoDateStrings(photo.exifDate).some(s => s.includes(q))) return true;
  }
  return false;
}
