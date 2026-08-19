import { describe, it, expect } from 'vitest';
import { albumMatchesQuery, photoMatchesQuery, photoDateStrings } from '../src/lib/search-core';

describe('albumMatchesQuery', () => {
  const album = { title: 'Vjenčanje Ana & Ivan', description: 'Zagreb, lipanj', tags: ['wedding', 'ljeto'] };

  it('matches title, description and tags case-insensitively', () => {
    expect(albumMatchesQuery('ana', album)).toBe(true);
    expect(albumMatchesQuery('VJENČANJE', album)).toBe(true);
    expect(albumMatchesQuery('zagreb', album)).toBe(true);
    expect(albumMatchesQuery('wedd', album)).toBe(true);
  });

  it('rejects non-matches and empty queries', () => {
    expect(albumMatchesQuery('krštenje', album)).toBe(false);
    expect(albumMatchesQuery('', album)).toBe(false);
    expect(albumMatchesQuery('  ', album)).toBe(false);
  });

  it('handles albums without description/tags', () => {
    expect(albumMatchesQuery('x', { title: 'Y' })).toBe(false);
    expect(albumMatchesQuery('y', { title: 'Y' })).toBe(true);
  });
});

describe('photoMatchesQuery', () => {
  const photo = {
    filename: 'DSC_4821.jpg',
    camera: 'Canon EOS R5',
    exifDate: new Date(2025, 5, 14) // 14.06.2025 local time
  };

  it('matches filename fragments case-insensitively', () => {
    expect(photoMatchesQuery('dsc_48', photo)).toBe(true);
    expect(photoMatchesQuery('4821', photo)).toBe(true);
  });

  it('matches camera make/model', () => {
    expect(photoMatchesQuery('canon', photo)).toBe(true);
    expect(photoMatchesQuery('eos r5', photo)).toBe(true);
    expect(photoMatchesQuery('nikon', photo)).toBe(false);
  });

  it('matches both ISO and Croatian date spellings', () => {
    expect(photoMatchesQuery('2025-06-14', photo)).toBe(true);
    expect(photoMatchesQuery('14.06.2025', photo)).toBe(true);
    expect(photoMatchesQuery('2025-06', photo)).toBe(true);
    expect(photoMatchesQuery('2025-07-14', photo)).toBe(false);
  });

  it('handles missing camera/date', () => {
    const bare = { filename: 'a.jpg' };
    expect(photoMatchesQuery('a.jpg', bare)).toBe(true);
    expect(photoMatchesQuery('canon', bare)).toBe(false);
  });
});

describe('photoDateStrings', () => {
  it('pads day and month', () => {
    expect(photoDateStrings(new Date(2025, 0, 5))).toEqual(['2025-01-05', '05.01.2025']);
  });
});
