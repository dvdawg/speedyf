// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { detectPlatform, markPlatform } from './platform';
import { installTitlebar, WINDOW_DRAG_ATTR } from './titlebar';

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({}) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: () => Promise.resolve() }));

describe('detectPlatform', () => {
  it('reads the webview user agent', () => {
    expect(detectPlatform('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit')).toBe(
      'macos'
    );
    expect(detectPlatform('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/120')).toBe('windows');
    expect(detectPlatform('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit')).toBe('linux');
  });

  it('publishes it on the root element', () => {
    markPlatform(document.documentElement, 'windows');
    expect(document.documentElement.dataset.platform).toBe('windows');
  });
});

describe('installTitlebar', () => {
  let cleanup: (() => void) | undefined;
  afterEach(() => {
    cleanup?.();
    document.body.innerHTML = '';
    document.documentElement.removeAttribute('data-fullscreen');
  });

  const setup = (fullscreen = false) => {
    const win = {
      startDragging: vi.fn(() => Promise.resolve()),
      isFullscreen: vi.fn(() => Promise.resolve(fullscreen)),
      onResized: vi.fn((_handler: () => void) => Promise.resolve(() => undefined)),
    };
    const zoom = vi.fn(() => Promise.resolve());
    const bar = document.createElement('div');
    bar.setAttribute(WINDOW_DRAG_ATTR, '');
    const button = document.createElement('button');
    bar.append(button);
    document.body.append(bar);
    cleanup = installTitlebar(document.documentElement, win, zoom);
    return { win, zoom, bar, button };
  };
  const press = (target: Element, type: string, detail: number, x = 5) =>
    target.dispatchEvent(
      new MouseEvent(type, { bubbles: true, button: 0, detail, clientX: x, clientY: 5 })
    );

  it('drags the window from empty bar space only', () => {
    const { win, bar, button } = setup();
    press(button, 'mousedown', 1);
    expect(win.startDragging).not.toHaveBeenCalled();
    press(bar, 'mousedown', 1);
    expect(win.startDragging).toHaveBeenCalledTimes(1);
  });

  it('zooms on a double-click that stays put, as the system bar does', () => {
    const { zoom, bar, button } = setup();
    press(bar, 'mousedown', 2);
    press(bar, 'mouseup', 2);
    expect(zoom).toHaveBeenCalledTimes(1);

    press(bar, 'mousedown', 2, 5);
    press(bar, 'mouseup', 2, 40);
    expect(zoom).toHaveBeenCalledTimes(1);

    press(button, 'mousedown', 2);
    press(button, 'mouseup', 2);
    expect(zoom).toHaveBeenCalledTimes(1);
  });

  it('marks full screen so the strip drops the traffic-light inset', async () => {
    setup(true);
    await Promise.resolve();
    await Promise.resolve();
    expect(document.documentElement.hasAttribute('data-fullscreen')).toBe(true);
  });
});
