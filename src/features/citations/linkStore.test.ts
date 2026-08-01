import { afterEach, describe, expect, it, vi } from 'vitest';
import { HoverMachine } from './linkStore';

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
