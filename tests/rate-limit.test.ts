import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { createRateLimiter, albumKey, passwordLimiter, shareTokenLimiter } from '../src/lib/rate-limit';
import {
  isRateLimited,
  recordFailedAttempt,
  clearRateLimit,
  getRemainingAttempts
} from '../src/lib/rate-limit';

const IP = '198.51.100.42';

beforeEach(() => {
  vi.useFakeTimers();
  clearRateLimit(IP);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('rate limiting', () => {
  it('allows fresh clients', () => {
    expect(isRateLimited(IP)).toBe(false);
    expect(getRemainingAttempts(IP)).toBe(10);
  });

  it('blocks after 10 failed attempts', () => {
    for (let i = 0; i < 9; i++) recordFailedAttempt(IP);
    expect(isRateLimited(IP)).toBe(false);
    recordFailedAttempt(IP);
    expect(isRateLimited(IP)).toBe(true);
    expect(getRemainingAttempts(IP)).toBe(0);
  });

  it('resets after the 15-minute window expires', () => {
    for (let i = 0; i < 10; i++) recordFailedAttempt(IP);
    expect(isRateLimited(IP)).toBe(true);

    vi.advanceTimersByTime(15 * 60 * 1000 + 1000);
    expect(isRateLimited(IP)).toBe(false);
    expect(getRemainingAttempts(IP)).toBe(10);
  });

  it('clears on successful login', () => {
    for (let i = 0; i < 10; i++) recordFailedAttempt(IP);
    clearRateLimit(IP);
    expect(isRateLimited(IP)).toBe(false);
  });

  it('tracks clients independently', () => {
    for (let i = 0; i < 10; i++) recordFailedAttempt(IP);
    expect(isRateLimited(IP)).toBe(true);
    expect(isRateLimited('203.0.113.9')).toBe(false);
  });
});


describe('createRateLimiter', () => {
  it('instances do not share state', () => {
    const a = createRateLimiter({ maxAttempts: 2, windowMs: 1000 });
    const b = createRateLimiter({ maxAttempts: 2, windowMs: 1000 });
    a.recordFailedAttempt('k');
    a.recordFailedAttempt('k');
    expect(a.isRateLimited('k')).toBe(true);
    expect(b.isRateLimited('k')).toBe(false);
  });

  it('the shared password and share-token buckets are separate', () => {
    // A flood of wrong passwords must not switch share links off: any page
    // on the internet can post passwords on a visitor's behalf.
    const key = albumKey('198.51.100.7', '2025/Weddings/Ana-Ivan');
    for (let i = 0; i < 10; i++) passwordLimiter.recordFailedAttempt(key);
    expect(passwordLimiter.isRateLimited(key)).toBe(true);
    expect(shareTokenLimiter.isRateLimited(key)).toBe(false);
    passwordLimiter.clearRateLimit(key);
  });

  it('keys are per album, case-insensitively like album ids', () => {
    expect(albumKey('1.2.3.4', '2025/Weddings')).toBe('1.2.3.4|2025/weddings');
    expect(albumKey('1.2.3.4', 'a')).not.toBe(albumKey('1.2.3.4', 'b'));
  });

  it('sweeps expired keys before evicting when full', () => {
    const limiter = createRateLimiter({ maxAttempts: 3, windowMs: 1000, maxEntries: 3 });
    limiter.recordFailedAttempt('old-1');
    limiter.recordFailedAttempt('old-2');
    vi.advanceTimersByTime(2000);
    limiter.recordFailedAttempt('fresh-1');
    // Full: 3 keys. The next new key sweeps the two expired ones.
    limiter.recordFailedAttempt('fresh-2');
    expect(limiter.size()).toBe(2);
    expect(limiter.getRemainingAttempts('fresh-1')).toBe(2);
  });

  it('never grows past maxEntries under a flood of new keys', () => {
    const limiter = createRateLimiter({ maxAttempts: 3, windowMs: 60_000, maxEntries: 100 });
    for (let i = 0; i < 1000; i++) limiter.recordFailedAttempt(`2001:db8::${i}`);
    expect(limiter.size()).toBe(100);
    // The most recent keys survive; the oldest were dropped.
    expect(limiter.getRemainingAttempts('2001:db8::999')).toBe(2);
    expect(limiter.getRemainingAttempts('2001:db8::0')).toBe(3);
  });
});
