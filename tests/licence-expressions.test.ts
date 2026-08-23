import { describe, it, expect } from 'vitest';
import { acceptable, nativeSideVetted } from '../scripts/check-licences.mjs';

/**
 * The licence guard is only as good as its reading of an SPDX expression, and
 * the failure that matters is the quiet one: accepting a dependency that
 * carries an obligation nobody looked at. The first version of this parser did
 * exactly that — it stripped parentheses and split on OR, so
 * `(MIT OR Apache-2.0) AND Unicode-3.0` passed on the strength of `MIT` alone
 * while the Unicode term went unread.
 */
describe('SPDX expression acceptance', () => {
  it('accepts a single permissive licence', () => {
    expect(acceptable('MIT')).toBe(true);
  });

  it('accepts a choice where one alternative is permissive', () => {
    expect(acceptable('MIT OR Apache-2.0')).toBe(true);
    expect(acceptable('MIT OR GPL-3.0')).toBe(true);
  });

  it('rejects a choice where no alternative is permissive', () => {
    expect(acceptable('GPL-3.0 OR AGPL-3.0')).toBe(false);
  });

  it('requires every term of an AND, not just the first', () => {
    expect(acceptable('Apache-2.0 AND ISC')).toBe(true);
    expect(acceptable('MIT AND GPL-3.0')).toBe(false);
  });

  it('does not let a parenthesised choice hide the licence it is ANDed with', () => {
    // The exact shape carried by unicode-ident, and the bug this guard had.
    expect(acceptable('(MIT OR Apache-2.0) AND Unicode-3.0')).toBe(true);
    expect(acceptable('(MIT OR Apache-2.0) AND GPL-3.0')).toBe(false);
  });

  it('reads the historical slash form as a choice', () => {
    expect(acceptable('MIT/Apache-2.0')).toBe(true);
    expect(acceptable('GPL-2.0/GPL-3.0')).toBe(false);
  });

  it('treats an exception as part of the identifier', () => {
    expect(acceptable('Apache-2.0 WITH LLVM-exception')).toBe(true);
    // A licence we allow, under an exception we have never considered, is not
    // something to wave through on the strength of the base identifier.
    expect(acceptable('Apache-2.0 WITH Some-Unreviewed-exception')).toBe(false);
  });

  it('refuses to vouch for an expression it cannot parse', () => {
    expect(acceptable('')).toBe(false);
    expect(acceptable('MIT Apache-2.0')).toBe(false);
    expect(acceptable('AND MIT')).toBe(false);
  });
});

describe('the native side of a -sys crate', () => {
  // cargo metadata describes the *binding*, not the C library behind it:
  // libheif-rs is MIT while libheif itself is LGPL-3.0. A plain licence check
  // waved that wrapper through — which is exactly how copyleft C code would
  // have shipped inside a "all permissive" tree. Native code enters through
  // -sys crates, so an unvetted one must fail loudly.
  it('refuses a -sys crate nobody has vetted, whatever its binding licence', () => {
    expect(nativeSideVetted('libheif-sys')).toBe(false);
    expect(nativeSideVetted('ffmpeg-sys')).toBe(false);
  });

  it('accepts the vetted ones and everything that links no native code', () => {
    expect(nativeSideVetted('libsqlite3-sys')).toBe(true);   // bundled SQLite: public domain
    expect(nativeSideVetted('security-framework-sys')).toBe(true); // OS-provided
    expect(nativeSideVetted('serde')).toBe(true);
    expect(nativeSideVetted('image')).toBe(true);
  });
});
