/**
 * Simple in-memory rate limiter for password attempts
 *
 * SECURITY: Prevents brute force attacks on password-protected albums
 */

interface RateLimitEntry {
  attempts: number;
  firstAttempt: number;
}

// In-memory store (resets on server restart)
const attempts = new Map<string, RateLimitEntry>();

// Configuration
const MAX_ATTEMPTS = 10;
const WINDOW_MS = 15 * 60 * 1000; // 15 minutes

/**
 * Check if an IP is rate limited
 * @param ip - Client IP address
 * @returns true if blocked, false if allowed
 */
export function isRateLimited(ip: string): boolean {
  const now = Date.now();
  const entry = attempts.get(ip);

  if (!entry) {
    return false;
  }

  // Check if window has expired
  if (now - entry.firstAttempt > WINDOW_MS) {
    attempts.delete(ip);
    return false;
  }

  return entry.attempts >= MAX_ATTEMPTS;
}

/**
 * Record a failed password attempt
 * @param ip - Client IP address
 */
export function recordFailedAttempt(ip: string): void {
  const now = Date.now();
  const entry = attempts.get(ip);

  if (!entry || now - entry.firstAttempt > WINDOW_MS) {
    // Start new window
    attempts.set(ip, { attempts: 1, firstAttempt: now });
  } else {
    // Increment existing
    entry.attempts++;
  }
}

/**
 * Clear rate limit for an IP (on successful login)
 * @param ip - Client IP address
 */
export function clearRateLimit(ip: string): void {
  attempts.delete(ip);
}

/**
 * Get remaining attempts for an IP
 * @param ip - Client IP address
 * @returns Number of remaining attempts
 */
export function getRemainingAttempts(ip: string): number {
  const entry = attempts.get(ip);
  if (!entry) return MAX_ATTEMPTS;

  const now = Date.now();
  if (now - entry.firstAttempt > WINDOW_MS) {
    return MAX_ATTEMPTS;
  }

  return Math.max(0, MAX_ATTEMPTS - entry.attempts);
}
