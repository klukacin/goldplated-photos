/**
 * Client proofing — pure validation logic (unit-tested).
 *
 * A proofing submission is a client's photo selection with optional per-photo
 * comments and an optional client name. Submissions are validated against the
 * album's actual photo list and hard limits before anything touches disk.
 */

export const PROOFING_LIMITS = {
  maxSelections: 500,
  maxCommentLength: 500,
  maxNameLength: 100
} as const;

export interface ProofingSelection {
  filename: string;
  comment: string | null;
}

export interface ProofingSubmission {
  submittedAt: string;
  name: string | null;
  selections: ProofingSelection[];
}

export type ProofingValidation =
  | { ok: true; name: string | null; selections: ProofingSelection[] }
  | { ok: false; error: string };

/**
 * Validate a raw request body against the album's real photo filenames.
 * Unknown filenames are rejected (not silently dropped) — a mismatch means
 * the client state is stale and the user should refresh.
 */
export function validateProofingPayload(
  body: unknown,
  knownFilenames: string[]
): ProofingValidation {
  if (typeof body !== 'object' || body === null) {
    return { ok: false, error: 'Invalid payload' };
  }
  const { name, selections } = body as { name?: unknown; selections?: unknown };

  let cleanName: string | null = null;
  if (name !== undefined && name !== null) {
    if (typeof name !== 'string') {
      return { ok: false, error: 'Invalid name' };
    }
    cleanName = name.trim().slice(0, PROOFING_LIMITS.maxNameLength) || null;
  }

  if (!Array.isArray(selections) || selections.length === 0) {
    return { ok: false, error: 'No photos selected' };
  }
  if (selections.length > PROOFING_LIMITS.maxSelections) {
    return { ok: false, error: `Too many selections (max ${PROOFING_LIMITS.maxSelections})` };
  }

  const known = new Set(knownFilenames);
  const seen = new Set<string>();
  const clean: ProofingSelection[] = [];

  for (const entry of selections) {
    if (typeof entry !== 'object' || entry === null) {
      return { ok: false, error: 'Invalid selection entry' };
    }
    const { filename, comment } = entry as { filename?: unknown; comment?: unknown };
    if (typeof filename !== 'string' || !filename) {
      return { ok: false, error: 'Invalid selection entry' };
    }
    if (!known.has(filename)) {
      return { ok: false, error: `Unknown photo: ${filename}` };
    }
    if (seen.has(filename)) {
      continue; // ignore duplicates
    }
    seen.add(filename);

    let cleanComment: string | null = null;
    if (comment !== undefined && comment !== null) {
      if (typeof comment !== 'string') {
        return { ok: false, error: 'Invalid comment' };
      }
      cleanComment = comment.trim().slice(0, PROOFING_LIMITS.maxCommentLength) || null;
    }
    clean.push({ filename, comment: cleanComment });
  }

  if (clean.length === 0) {
    return { ok: false, error: 'No photos selected' };
  }

  return { ok: true, name: cleanName, selections: clean };
}

/** Build a filesystem-safe submission filename: <iso-ts>-<name-slug>.json */
export function submissionFilename(submittedAt: Date, name: string | null): string {
  const timestamp = submittedAt.toISOString().replace(/[:.]/g, '-');
  const slug = (name || 'anonymous')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40) || 'anonymous';
  return `${timestamp}-${slug}.json`;
}
