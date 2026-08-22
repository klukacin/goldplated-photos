/**
 * Photo/video info overlay rendering — pure string building (unit-tested).
 *
 * These two functions turn the payloads of `/api/exif` and `/api/video-info`
 * into the markup the lightbox drops into an overlay with `innerHTML`.
 *
 * SECURITY: their inputs are not the photographer's words. EXIF strings such
 * as Make, Model and LensModel are carried inside the image file, so any photo
 * that reaches an album — from a second shooter, a client, a stock download —
 * can put arbitrary text in them. Every interpolated value therefore goes
 * through `escapeHtml`; only the fixed labels and titles in this file are
 * written into the markup as-is.
 */

/** Escape the five characters that can break out of text or an attribute. */
export function escapeHtml(value: unknown): string {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

interface InfoField {
  label: string;
  value: unknown;
}

interface InfoSection {
  title?: string;
  fields?: InfoField[];
  isRating?: boolean;
  stars?: string;
}

/** Render the shared `<dl>` block for one section of fields. */
function renderFields(section: InfoSection): string {
  let html = `
          <div class="exif-section" style="margin-bottom: 1.5rem;">
            <h4 style="margin-bottom: 0.75rem; color: #333; font-size: 1.1rem;">${section.title}</h4>
            <dl style="display: grid; grid-template-columns: auto 1fr; gap: 0.5rem 1rem; margin: 0;">
        `;
  (section.fields || []).forEach(field => {
    html += `
            <dt style="font-weight: 500; color: #666;">${field.label}</dt>
            <dd style="color: #333; margin: 0;">${escapeHtml(field.value)}</dd>
          `;
  });
  html += `</dl></div>`;
  return html;
}

/** Build the EXIF overlay markup from an `/api/exif` payload. */
export function formatExifData(exif: any): string {
  const sections: InfoSection[] = [];

  // Rating section - show first if available (from XMP metadata)
  if (exif.Rating !== undefined && exif.Rating > 0) {
    const rating = Math.min(5, Math.max(0, Math.round(exif.Rating)));
    const stars = '★'.repeat(rating) + '☆'.repeat(5 - rating);
    sections.push({
      isRating: true,
      stars: stars
    });
  }

  // Camera section
  const cameraFields: InfoField[] = [];
  if (exif.Make) cameraFields.push({ label: 'Make', value: exif.Make });
  if (exif.Model) cameraFields.push({ label: 'Model', value: exif.Model });
  if (exif.LensModel || exif.Lens) cameraFields.push({ label: 'Lens', value: exif.LensModel || exif.Lens });

  if (cameraFields.length > 0) {
    sections.push({
      title: '📷 Camera',
      fields: cameraFields
    });
  }

  // Settings section
  const settingsFields: InfoField[] = [];
  if (exif.ISO) settingsFields.push({ label: 'ISO', value: exif.ISO });
  if (exif.FNumber) settingsFields.push({ label: 'Aperture', value: `f/${exif.FNumber}` });
  if (exif.ExposureTime) {
    const shutterSpeed = exif.ExposureTime < 1
      ? `1/${Math.round(1/exif.ExposureTime)}s`
      : `${exif.ExposureTime}s`;
    settingsFields.push({ label: 'Shutter Speed', value: shutterSpeed });
  }
  if (exif.FocalLength) settingsFields.push({ label: 'Focal Length', value: `${exif.FocalLength}mm` });
  if (exif.ExposureCompensation) settingsFields.push({ label: 'Exposure Comp', value: `${exif.ExposureCompensation > 0 ? '+' : ''}${exif.ExposureCompensation} EV` });

  if (settingsFields.length > 0) {
    sections.push({
      title: '⚙️ Settings',
      fields: settingsFields
    });
  }

  // Image info section
  const imageFields: InfoField[] = [];
  if (exif.ImageWidth && exif.ImageHeight) {
    imageFields.push({ label: 'Dimensions', value: `${exif.ImageWidth} × ${exif.ImageHeight}` });
  }
  if (exif.DateTimeOriginal || exif.DateTime) {
    const dateStr = exif.DateTimeOriginal || exif.DateTime;
    imageFields.push({ label: 'Date Taken', value: dateStr });
  }
  if (exif.Flash !== undefined) {
    imageFields.push({ label: 'Flash', value: exif.Flash ? 'Yes' : 'No' });
  }

  if (imageFields.length > 0) {
    sections.push({
      title: 'ℹ️ Image',
      fields: imageFields
    });
  }

  // Location section
  if (exif.GPSLatitude && exif.GPSLongitude) {
    const locationFields: InfoField[] = [
      { label: 'Latitude', value: exif.GPSLatitude },
      { label: 'Longitude', value: exif.GPSLongitude }
    ];
    if (exif.GPSAltitude) {
      locationFields.push({ label: 'Altitude', value: `${exif.GPSAltitude}m` });
    }
    sections.push({
      title: '📍 Location',
      fields: locationFields
    });
  }

  if (sections.length === 0) {
    return '<p style="color: #666;">No detailed EXIF data available.</p>';
  }

  let html = '';
  sections.forEach(section => {
    if (section.isRating) {
      // Rating display - elegant gold stars, centered
      html += `
          <div class="exif-rating" style="
            text-align: center;
            margin-bottom: 1.25rem;
            padding-bottom: 1rem;
            border-bottom: 1px solid #eee;
          ">
            <span class="rating-stars" style="
              font-size: 1.4rem;
              letter-spacing: 3px;
              color: #d4af37;
              text-shadow: 0 1px 2px rgba(0,0,0,0.08);
            ">${section.stars}</span>
          </div>
        `;
    } else {
      html += renderFields(section);
    }
  });

  return html;
}

/** Build the video-info overlay markup from an `/api/video-info` payload. */
export function formatVideoInfo(info: any): string {
  const sections: InfoSection[] = [];

  // Video section
  const videoFields: InfoField[] = [];
  if (info.duration) videoFields.push({ label: 'Duration', value: info.duration });
  if (info.resolution) videoFields.push({ label: 'Resolution', value: info.resolution });
  if (info.codec) videoFields.push({ label: 'Video Codec', value: info.codec });
  if (info.frameRate) videoFields.push({ label: 'Frame Rate', value: info.frameRate });

  if (videoFields.length > 0) {
    sections.push({ title: '🎬 Video', fields: videoFields });
  }

  // Audio section
  const audioFields: InfoField[] = [];
  if (info.audioCodec) audioFields.push({ label: 'Audio Codec', value: info.audioCodec });

  if (audioFields.length > 0) {
    sections.push({ title: '🔊 Audio', fields: audioFields });
  }

  // File section
  const fileFields: InfoField[] = [];
  if (info.size) fileFields.push({ label: 'File Size', value: info.size });
  if (info.bitrate) fileFields.push({ label: 'Bitrate', value: info.bitrate });

  if (fileFields.length > 0) {
    sections.push({ title: '📁 File', fields: fileFields });
  }

  if (sections.length === 0) {
    return '<p style="color: #666;">No video information available.</p>';
  }

  let html = '';
  sections.forEach(section => {
    html += renderFields(section);
  });

  return html;
}
