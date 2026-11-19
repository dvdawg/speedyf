/** Which desktop the shell runs on, for the few places the chrome differs.
 *
 * Read from the user agent once: WKWebView says Macintosh, WebView2 says
 * Windows, WebKitGTK says X11 or Wayland on Linux. Published on the root
 * element as `data-platform` so stylesheets can gate on it. */
export type Platform = 'macos' | 'windows' | 'linux';

export function detectPlatform(userAgent: string): Platform {
  if (/mac/i.test(userAgent)) return 'macos';
  if (/win/i.test(userAgent)) return 'windows';
  return 'linux';
}

export const platform: Platform = detectPlatform(
  typeof navigator === 'undefined' ? '' : navigator.userAgent
);

export function markPlatform(root: HTMLElement, which: Platform = platform) {
  root.dataset.platform = which;
}
