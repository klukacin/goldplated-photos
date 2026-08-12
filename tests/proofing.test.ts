import { describe, it, expect } from 'vitest';
import { validateProofingPayload, submissionFilename, PROOFING_LIMITS } from '../src/lib/proofing';

const KNOWN = ['a.jpg', 'b.jpg', 'c.jpg'];

describe('validateProofingPayload', () => {
  it('accepts a valid submission and trims values', () => {
    const result = validateProofingPayload({
      name: '  Ana Anić  ',
      selections: [
        { filename: 'a.jpg', comment: '  crno-bijela molim  ' },
        { filename: 'b.jpg' }
      ]
    }, KNOWN);
    expect(result).toEqual({
      ok: true,
      name: 'Ana Anić',
      selections: [
        { filename: 'a.jpg', comment: 'crno-bijela molim' },
        { filename: 'b.jpg', comment: null }
      ]
    });
  });

  it('rejects unknown filenames (stale client state)', () => {
    const result = validateProofingPayload({ selections: [{ filename: 'nope.jpg' }] }, KNOWN);
    expect(result.ok).toBe(false);
  });

  it('rejects empty and non-array selections', () => {
    expect(validateProofingPayload({ selections: [] }, KNOWN).ok).toBe(false);
    expect(validateProofingPayload({ selections: 'a.jpg' }, KNOWN).ok).toBe(false);
    expect(validateProofingPayload(null, KNOWN).ok).toBe(false);
    expect(validateProofingPayload('x', KNOWN).ok).toBe(false);
  });

  it('rejects oversized selections and malformed entries', () => {
    const tooMany = Array.from({ length: PROOFING_LIMITS.maxSelections + 1 }, () => ({ filename: 'a.jpg' }));
    expect(validateProofingPayload({ selections: tooMany }, KNOWN).ok).toBe(false);
    expect(validateProofingPayload({ selections: [{ filename: 42 }] }, KNOWN).ok).toBe(false);
    expect(validateProofingPayload({ selections: [{ filename: 'a.jpg', comment: 42 }] }, KNOWN).ok).toBe(false);
    expect(validateProofingPayload({ name: 42, selections: [{ filename: 'a.jpg' }] }, KNOWN).ok).toBe(false);
  });

  it('deduplicates repeated filenames and caps comment/name length', () => {
    const longComment = 'x'.repeat(PROOFING_LIMITS.maxCommentLength + 100);
    const longName = 'y'.repeat(PROOFING_LIMITS.maxNameLength + 50);
    const result = validateProofingPayload({
      name: longName,
      selections: [
        { filename: 'a.jpg', comment: longComment },
        { filename: 'a.jpg', comment: 'duplicate' }
      ]
    }, KNOWN);
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.selections).toHaveLength(1);
      expect(result.selections[0].comment).toHaveLength(PROOFING_LIMITS.maxCommentLength);
      expect(result.name).toHaveLength(PROOFING_LIMITS.maxNameLength);
    }
  });
});

describe('submissionFilename', () => {
  it('builds a filesystem-safe name from timestamp and client name', () => {
    const name = submissionFilename(new Date('2026-08-12T10:30:00Z'), 'Ana / Anić!');
    expect(name).toBe('2026-08-12T10-30-00-000Z-ana-ani.json');
  });
  it('falls back to anonymous', () => {
    expect(submissionFilename(new Date('2026-08-12T10:30:00Z'), null)).toMatch(/-anonymous\.json$/);
    expect(submissionFilename(new Date('2026-08-12T10:30:00Z'), '!!!')).toMatch(/-anonymous\.json$/);
  });
});
