/** Window frames across launches, and showing a window only once it has
 * painted.
 *
 * Each window remembers its own frame beside its tabs (sessionStore). The
 * main window applies its frame itself before it is shown; a window reopened
 * from the last launch is built by Rust at its frame. Either way the frame
 * is first checked against the screens attached now (`fitToScreens`): a
 * window last seen on a monitor that has since been unplugged must not come
 * back somewhere nobody can reach it. */
import {
  availableMonitors,
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from '@tauri-apps/api/window';
import { saveBounds, type WindowBounds } from '../../stores/sessionStore';
import { desktop } from '../../lib/transport/desktop';

/** A screen's usable area (without the menu bar, Dock or taskbar), in
 * logical pixels. The first is the primary screen. */
export interface WorkArea {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The strip along a window's top edge it is dragged by. */
const TITLE_BAR = 32;
/** How much of that strip must be on some screen for the window to count as
 * reachable where it is. */
const MIN_VISIBLE = 80;
/** The smallest window the app allows (tauri.conf.json). */
const MIN_SIZE = { width: 720, height: 480 };

function overlap(a0: number, a1: number, b0: number, b1: number): number {
  return Math.max(0, Math.min(a1, b1) - Math.max(a0, b0));
}

/** `bounds`, moved and shrunk as little as possible so the window can be
 * reached on the screens in `areas`.
 *
 * A window whose title bar is on a screen stays where it is - hanging off an
 * edge is the user's choice - but is no larger than that screen and never
 * above its top, where the title bar would be out of reach. A window on no
 * screen at all (its monitor is gone) is centred on the primary one. With no
 * screens to go by the frame is left alone. Pure. */
export function fitToScreens(bounds: WindowBounds, areas: readonly WorkArea[]): WindowBounds {
  if (areas.length === 0) return bounds;
  let best: WorkArea | null = null;
  let bestVisible = 0;
  for (const area of areas) {
    const wide = overlap(bounds.x, bounds.x + bounds.width, area.x, area.x + area.width);
    const tall = overlap(bounds.y, bounds.y + TITLE_BAR, area.y, area.y + area.height);
    const visible = tall > 0 ? wide : 0;
    if (visible > bestVisible) {
      best = area;
      bestVisible = visible;
    }
  }
  const reachable = best !== null && bestVisible >= Math.min(MIN_VISIBLE, bounds.width);
  const home = reachable ? best! : areas[0]!;
  const width = Math.max(Math.min(bounds.width, home.width), Math.min(MIN_SIZE.width, home.width));
  const height = Math.max(
    Math.min(bounds.height, home.height),
    Math.min(MIN_SIZE.height, home.height)
  );
  if (reachable) {
    const y = Math.min(Math.max(bounds.y, home.y), home.y + home.height - TITLE_BAR);
    return { ...bounds, y, width, height };
  }
  return {
    ...bounds,
    x: Math.round(home.x + (home.width - width) / 2),
    y: Math.round(home.y + (home.height - height) / 2),
    width,
    height,
  };
}

/** The screens attached now, in logical pixels. Empty when that cannot be
 * known, which leaves frames as they were saved. */
export async function workAreas(): Promise<WorkArea[]> {
  try {
    const monitors = await availableMonitors();
    return monitors.map((m) => ({
      x: m.workArea.position.x / m.scaleFactor,
      y: m.workArea.position.y / m.scaleFactor,
      width: m.workArea.size.width / m.scaleFactor,
      height: m.workArea.size.height / m.scaleFactor,
    }));
  } catch {
    return [];
  }
}

/** Puts this window at `bounds` (already fitted to the screens). */
export async function applyBounds(bounds: WindowBounds): Promise<void> {
  const win = getCurrentWindow();
  try {
    await win.setSize(new LogicalSize(bounds.width, bounds.height));
    await win.setPosition(new LogicalPosition(bounds.x, bounds.y));
    if (bounds.maximized) await win.maximize();
  } catch {
    // The window keeps its default frame.
  }
}

/** The frame a maximized or full-screen window returns to: the last one it
 * had while it was neither. */
let lastNormal: WindowBounds | null = null;

async function currentBounds(): Promise<WindowBounds | null> {
  const win = getCurrentWindow();
  try {
    const [maximized, fullscreen, minimized] = await Promise.all([
      win.isMaximized(),
      win.isFullscreen(),
      win.isMinimized(),
    ]);
    if (minimized) return null;
    if (maximized || fullscreen) {
      return lastNormal ? { ...lastNormal, maximized } : null;
    }
    const [scale, position, size] = await Promise.all([
      win.scaleFactor(),
      win.outerPosition(),
      win.innerSize(),
    ]);
    lastNormal = {
      x: Math.round(position.x / scale),
      y: Math.round(position.y / scale),
      width: Math.round(size.width / scale),
      height: Math.round(size.height / scale),
      maximized: false,
    };
    return lastNormal;
  } catch {
    return null;
  }
}

/** Records this window's frame now: before it closes, and when it has
 * stopped moving. */
export async function recordBounds(label: string): Promise<void> {
  const bounds = await currentBounds();
  if (bounds) saveBounds(bounds, label);
}

/** Keeps this window's saved frame current as it is moved and resized. */
export function trackBounds(label: string): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const settle = () => {
    clearTimeout(timer);
    timer = setTimeout(() => void recordBounds(label), 400);
  };
  const unlisteners: (() => void)[] = [];
  let disposed = false;
  const win = getCurrentWindow();
  for (const subscribe of [win.onMoved.bind(win), win.onResized.bind(win)]) {
    void subscribe(settle)
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      })
      .catch(() => undefined);
  }
  void recordBounds(label);
  return () => {
    disposed = true;
    clearTimeout(timer);
    for (const unlisten of unlisteners) unlisten();
  };
}

/** Shows this window, built hidden, once it has painted: no white frame on
 * launch or for a new window. A hidden webview may hold its animation
 * frames back, so a short timer stands in for them; Rust shows the window
 * after a longer grace period should neither ever run. */
export function revealWhenPainted(): void {
  let done = false;
  const reveal = () => {
    if (done) return;
    done = true;
    void desktop.revealWindow();
  };
  requestAnimationFrame(() => requestAnimationFrame(reveal));
  setTimeout(reveal, 150);
}
