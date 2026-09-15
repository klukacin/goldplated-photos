/**
 * In-memory rate limiting for anything a stranger can retry: password
 * attempts, share-token guesses, proofing submissions, sync bearer tokens.
 *
 * SECURITY: each limiter is a fixed window per key. Keys are caller-chosen —
 * the password limiter uses `ip|albumPath` so a burst forced on one album
 * (a cross-site auto-submitting form, say) does not lock every album for
 * that IP and everyone behind the same NAT.
 *
 * The map is bounded: an attacker rotating through a /64 of IPv6 addresses
 * must not be able to grow it without limit. When it fills, expired entries
 * are swept; if it is still full, the oldest entry is dropped (Map iteration
 * order is insertion order).
 */

interface RateLimitEntry {
  attempts: number;
  firstAttempt: number;
}

export interface RateLimiterOptions {
  maxAttempts: number;
  windowMs: number;
  /** Upper bound on tracked keys (default 10 000). */
  maxEntries?: number;
}

export interface RateLimiter {
  /** true when the key has exhausted its attempts inside the current window. */
  isRateLimited(key: string): boolean;
  /** Record one failed attempt for the key. */
  recordFailedAttempt(key: string): void;
  /** Forget the key (on success). */
  clearRateLimit(key: string): void;
  /** Attempts left in the current window. */
  getRemainingAttempts(key: string): number;
  /** Number of keys currently tracked (for tests). */
  size(): number;
  /** Drop everything (for tests). */
  _reset(): void;
}

const DEFAULT_MAX_ENTRIES = 10_000;

export function createRateLimiter(options: RateLimiterOptions): RateLimiter {
  const { maxAttempts, windowMs } = options;
  const maxEntries = options.maxEntries ?? DEFAULT_MAX_ENTRIES;
  const attempts = new Map<string, RateLimitEntry>();

  const expired = (entry: RateLimitEntry, now: number) => now - entry.firstAttempt > windowMs;

  function sweep(now: number): void {
    for (const [key, entry] of attempts) {
      if (expired(entry, now)) attempts.delete(key);
    }
  }

  function makeRoom(now: number): void {
    if (attempts.size < maxEntries) return;
    sweep(now);
    // Still full: evict the oldest tracked keys until one slot is free. Losing
    // the oldest window is the cheapest wrong answer — it lets one early
    // attacker retry sooner, rather than letting a flood exhaust memory.
    while (attempts.size >= maxEntries) {
      const oldest = attempts.keys().next().value;
      if (oldest === undefined) break;
      attempts.delete(oldest);
    }
  }

  return {
    isRateLimited(key) {
      const now = Date.now();
      const entry = attempts.get(key);
      if (!entry) return false;
      if (expired(entry, now)) {
        attempts.delete(key);
        return false;
      }
      return entry.attempts >= maxAttempts;
    },

    recordFailedAttempt(key) {
      const now = Date.now();
      const entry = attempts.get(key);
      if (!entry || expired(entry, now)) {
        if (!entry) makeRoom(now);
        attempts.set(key, { attempts: 1, firstAttempt: now });
      } else {
        entry.attempts++;
      }
    },

    clearRateLimit(key) {
      attempts.delete(key);
    },

    getRemainingAttempts(key) {
      const entry = attempts.get(key);
      if (!entry) return maxAttempts;
      if (expired(entry, Date.now())) return maxAttempts;
      return Math.max(0, maxAttempts - entry.attempts);
    },

    size: () => attempts.size,
    _reset: () => attempts.clear()
  };
}

// ---------------------------------------------------------------------------
// Shared instances
// ---------------------------------------------------------------------------

const FIFTEEN_MINUTES = 15 * 60 * 1000;

/** Password attempts: 10 per 15 minutes, keyed by `ip|albumPath`. */
export const passwordLimiter = createRateLimiter({ maxAttempts: 10, windowMs: FIFTEEN_MINUTES });

/**
 * Share-token guesses: a separate bucket, so a password flood cannot switch
 * share links off for the same IP. The token is 128 bits; this only bounds
 * the request rate.
 */
export const shareTokenLimiter = createRateLimiter({ maxAttempts: 10, windowMs: FIFTEEN_MINUTES });

/** Build the per-album key both password and token limiters use. */
export function albumKey(ip: string, albumPath: string): string {
  return `${ip}|${albumPath.toLowerCase()}`;
}

// Backwards-compatible named exports (the password limiter).
export const isRateLimited = passwordLimiter.isRateLimited;
export const recordFailedAttempt = passwordLimiter.recordFailedAttempt;
export const clearRateLimit = passwordLimiter.clearRateLimit;
export const getRemainingAttempts = passwordLimiter.getRemainingAttempts;
