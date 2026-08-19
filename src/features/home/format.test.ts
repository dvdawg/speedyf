import { describe, expect, it } from 'vitest';
import { formatDate, formatSize } from './format';

describe('formatSize', () => {
  it('renders an en dash when the size is unknown', () => {
    expect(formatSize(null)).toBe('–');
  });

  it('reports raw bytes below one kibibyte', () => {
    expect(formatSize(0)).toBe('0 B');
    expect(formatSize(1023)).toBe('1023 B');
  });

  it('scales through KB, MB and GB', () => {
    expect(formatSize(1024)).toBe('1.0 KB');
    expect(formatSize(1024 ** 2)).toBe('1.0 MB');
    expect(formatSize(1024 ** 3)).toBe('1.0 GB');
  });

  // GB is the largest unit, so the loop stops there rather than reaching TB.
  it('stays in GB past a terabyte', () => {
    expect(formatSize(1024 ** 4)).toBe('1024 GB');
  });

  it('keeps one decimal below ten and drops it at or above ten', () => {
    expect(formatSize(1536)).toBe('1.5 KB');
    expect(formatSize(9.5 * 1024)).toBe('9.5 KB');
    expect(formatSize(10 * 1024)).toBe('10 KB');
    expect(formatSize(512 * 1024)).toBe('512 KB');
  });
});

describe('formatDate', () => {
  it('renders a time for a timestamp earlier today', () => {
    const today = new Date();
    today.setHours(14, 5, 0, 0);
    const formatted = formatDate(today.getTime());
    expect(formatted).toBe(
      today.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
    );
  });

  it('renders a calendar date for any other day', () => {
    const past = new Date();
    past.setFullYear(past.getFullYear() - 1);
    const formatted = formatDate(past.getTime());
    expect(formatted).toBe(
      past.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
    );
  });

  // Same clock time, different day — must not be mistaken for "today".
  it('distinguishes a day boundary from a matching wall-clock time', () => {
    const now = new Date();
    const yesterday = new Date(now.getTime() - 24 * 60 * 60 * 1000);
    expect(formatDate(yesterday.getTime())).not.toBe(formatDate(now.getTime()));
  });
});
