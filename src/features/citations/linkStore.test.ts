import { afterEach, describe, expect, it, vi } from 'vitest';
import { HoverMachine, ReferenceNavigationHistory } from './linkStore';

interface Request {
  key: string;
}

const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};

afterEach(() => {
  vi.useRealTimers();
});

describe('HoverMachine', () => {
  it('suppresses engine work when the pointer leaves before dwell', () => {
    vi.useFakeTimers();
    const loader = vi.fn(async (request: Request) => request.key);
    const machine = new HoverMachine(loader);

    machine.enter({ key: 'a' });
    vi.advanceTimersByTime(100);
    machine.leave();
    vi.advanceTimersByTime(120);

    expect(loader).not.toHaveBeenCalled();
    expect(machine.snapshot().phase).toBe('idle');
  });

  it('cancels the leave grace period when the popover is entered', async () => {
    vi.useFakeTimers();
    const machine = new HoverMachine(async (request: Request) => request.key);
    machine.enter({ key: 'a' }, true);
    vi.advanceTimersByTime(0);
    await flush();
    expect(machine.snapshot().phase).toBe('shown');

    machine.leave();
    vi.advanceTimersByTime(60);
    machine.enterPopover();
    vi.advanceTimersByTime(200);

    expect(machine.snapshot().phase).toBe('shown');
    expect(machine.snapshot().result).toBe('a');
  });

  it('drops stale replies and serializes the replacement request', async () => {
    vi.useFakeTimers();
    let resolveA!: (value: string) => void;
    let resolveB!: (value: string) => void;
    const loader = vi.fn(
      (request: Request) =>
        new Promise<string>((resolve) => {
          if (request.key === 'a') resolveA = resolve;
          else resolveB = resolve;
        })
    );
    const machine = new HoverMachine(loader);

    machine.enter({ key: 'a' }, true);
    vi.advanceTimersByTime(0);
    expect(loader).toHaveBeenCalledTimes(1);
    machine.enter({ key: 'b' }, true);
    vi.advanceTimersByTime(0);
    expect(loader).toHaveBeenCalledTimes(1);

    resolveA('stale-a');
    await flush();
    expect(loader).toHaveBeenCalledTimes(2);
    resolveB('fresh-b');
    await flush();

    expect(machine.snapshot().request?.key).toBe('b');
    expect(machine.snapshot().result).toBe('fresh-b');
  });

  it('shows a memoized preview immediately on re-hover', async () => {
    vi.useFakeTimers();
    const loader = vi.fn(async (request: Request) => `preview-${request.key}`);
    const machine = new HoverMachine(loader);
    machine.enter({ key: 'a' }, true);
    vi.advanceTimersByTime(0);
    await flush();
    machine.close();

    machine.enter({ key: 'a' });

    expect(machine.snapshot().phase).toBe('shown');
    expect(machine.snapshot().result).toBe('preview-a');
    expect(loader).toHaveBeenCalledTimes(1);
  });
});

describe('ReferenceNavigationHistory', () => {
  it('unwinds chained reference jumps in reverse order', () => {
    const history = new ReferenceNavigationHistory();
    history.push({ docId: 7, scrollTop: 120, scrollLeft: 4 });
    history.push({ docId: 7, scrollTop: 980, scrollLeft: 0 });

    expect(history.depth).toBe(2);
    expect(history.pop(7)).toEqual({ docId: 7, scrollTop: 980, scrollLeft: 0 });
    expect(history.pop(7)).toEqual({ docId: 7, scrollTop: 120, scrollLeft: 4 });
    expect(history.depth).toBe(0);
  });

  it('does not restore positions belonging to a different document', () => {
    const history = new ReferenceNavigationHistory();
    history.push({ docId: 2, scrollTop: 300, scrollLeft: 10 });

    expect(history.pop(3)).toBeUndefined();
    expect(history.depth).toBe(0);
  });

  it('bounds retained reading positions', () => {
    const history = new ReferenceNavigationHistory(2);
    history.push({ docId: 1, scrollTop: 10, scrollLeft: 0 });
    history.push({ docId: 1, scrollTop: 20, scrollLeft: 0 });
    history.push({ docId: 1, scrollTop: 30, scrollLeft: 0 });

    expect(history.depth).toBe(2);
    expect(history.pop(1)?.scrollTop).toBe(30);
    expect(history.pop(1)?.scrollTop).toBe(20);
  });
});
