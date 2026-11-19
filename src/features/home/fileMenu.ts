/** The right-click menu on a row that stands for a file - a recent file, a
 * library search result, an item on the Projects screen: open it here or in
 * a window of its own, show it where it lives on disk, and whatever the
 * row's own list can do to it (forget it, take it out of a project). */
import { SEPARATOR, type MenuEntry } from '../../lib/nativeMenu';
import { IS_MAC } from '../../lib/platform';
import { desktop } from '../../lib/transport/desktop';
import { showError } from '../../stores/modalStore';
import { openInNewWindow } from '../windows/windowController';

const IS_WINDOWS = typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent);

/** What the file manager is called here. */
export function revealLabel(mac: boolean = IS_MAC, windows: boolean = IS_WINDOWS): string {
  if (mac) return 'Show in Finder';
  if (windows) return 'Show in Explorer';
  return 'Open Containing Folder';
}

export function fileMenuEntries(
  path: string | null,
  opts: { open: () => void; extra?: readonly MenuEntry[] }
): MenuEntry[] {
  return [
    { text: 'Open', enabled: path !== null, run: opts.open },
    {
      text: 'Open in New Window',
      enabled: path !== null,
      run: () => {
        if (path) void openInNewWindow(path);
      },
    },
    SEPARATOR,
    {
      text: revealLabel(),
      enabled: path !== null,
      run: () => {
        if (path) desktop.revealInFileManager(path).catch((e) => showError(String(e)));
      },
    },
    ...(opts.extra && opts.extra.length > 0 ? [SEPARATOR, ...opts.extra] : []),
  ];
}
