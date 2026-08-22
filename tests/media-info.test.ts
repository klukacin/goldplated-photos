import { describe, it, expect } from 'vitest';
import { escapeHtml, formatExifData, formatVideoInfo } from '../src/lib/media-info';

const XSS = '<img src=x onerror="alert(1)">';

describe('escapeHtml', () => {
  it('neutralises the characters that break out of markup', () => {
    expect(escapeHtml(`<a href="x">'&'</a>`)).toBe(
      '&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;'
    );
  });

  it('leaves ordinary camera names alone', () => {
    expect(escapeHtml('NIKON Z 7_2')).toBe('NIKON Z 7_2');
  });
});

describe('formatExifData', () => {
  // EXIF strings travel inside the image file, so a photo handed over by a
  // client or a second shooter can carry markup in them. The overlay writes
  // this string with innerHTML, so an unescaped value is script execution on
  // the gallery's own origin — where the album-access cookie lives.
  it('escapes camera metadata rather than letting it become markup', () => {
    const html = formatExifData({ Make: XSS, Model: 'Z7', LensModel: XSS });
    // No tag ever opens, so `onerror` stays inert text rather than an attribute
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img src=x onerror=&quot;alert(1)&quot;&gt;');
    expect(html).toContain('Z7');
  });

  it('escapes the capture date, which is also file-supplied', () => {
    const html = formatExifData({ DateTimeOriginal: XSS });
    expect(html).not.toContain('<img src=x');
  });

  it('still renders real values readably', () => {
    const html = formatExifData({ Make: 'Canon', Model: 'EOS R5', ISO: 400, FNumber: 1.8 });
    expect(html).toContain('Canon');
    expect(html).toContain('EOS R5');
    expect(html).toContain('400');
    expect(html).toContain('f/1.8');
  });

  it('reports when there is nothing to show', () => {
    expect(formatExifData({})).toContain('No detailed EXIF data available.');
  });

  it('renders a clamped star rating', () => {
    expect(formatExifData({ Rating: 3 })).toContain('★★★☆☆');
    expect(formatExifData({ Rating: 99 })).toContain('★★★★★');
  });
});

describe('formatVideoInfo', () => {
  it('escapes ffprobe-derived values rather than letting them become markup', () => {
    const html = formatVideoInfo({ codec: XSS, resolution: '1920x1080' });
    expect(html).not.toContain('<img src=x');
    expect(html).toContain('1920x1080');
  });

  it('reports when there is nothing to show', () => {
    expect(formatVideoInfo({})).toContain('No video information available.');
  });
});
