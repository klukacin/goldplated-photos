import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
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
