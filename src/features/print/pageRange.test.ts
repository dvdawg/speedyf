import { describe, expect, it } from 'vitest';
import { checkPageRange } from './pageRange';

describe('checkPageRange', () => {
  it('accepts a single page, a span, and a list', () => {
    for (const value of ['1', '2-5', '1,4,7-9', '24']) {
      expect(checkPageRange(value, 24), value).toBeNull();
    }
  });

  it('rejects an empty range rather than silently printing everything', () => {
    expect(checkPageRange('', 24)?.message).toMatch(/Enter a range/);
    expect(checkPageRange('   ', 24)).not.toBeNull();
  });

  it('rejects open ends and backwards spans', () => {
    expect(checkPageRange('3-', 24)).not.toBeNull();
    expect(checkPageRange('-3', 24)).not.toBeNull();
    expect(checkPageRange('5-2', 24)?.message).toMatch(/backwards/);
  });

  it('rejects page zero and pages past the end', () => {
    expect(checkPageRange('0', 24)?.message).toMatch(/start at 1/);
    expect(checkPageRange('25', 24)?.message).toMatch(/24 pages/);
    expect(checkPageRange('1-99', 24)?.message).toMatch(/24 pages/);
  });

  it('rejects separators CUPS would not understand', () => {
    for (const value of ['1;2', '1 2', '1--2', 'a', '1-2-3']) {
      expect(checkPageRange(value, 24), value).not.toBeNull();
    }
  });
});
