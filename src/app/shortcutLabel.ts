/** Display text for keyboard shortcut specs (`mod+shift+s`). */

const IS_MAC = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

const MAC_PARTS: Record<string, string> = {
  mod: 'Cmd',
  ctrl: 'Ctrl',
  shift: 'Shift',
  alt: 'Option',
  plus: 'Plus',
  minus: 'Minus',
  backspace: 'Backspace',
  pagedown: 'PgDn',
  pageup: 'PgUp',
  tab: 'Tab',
};

const PC_PARTS: Record<string, string> = {
  ...MAC_PARTS,
  mod: 'Ctrl',
  alt: 'Alt',
};

/** Render a neutral shortcut spec for the current platform.
 *
 * Plain words joined with `+` everywhere (`Cmd+Shift+S` on macOS,
 * `Ctrl+Shift+S` elsewhere): no key glyphs, so the text reads the same in
 * every font. Storing the spec neutrally is what keeps a Windows build from
 * advertising a Cmd key that does not exist on the keyboard.
 */
export function formatShortcut(spec: string, isMac: boolean = IS_MAC): string {
  const table = isMac ? MAC_PARTS : PC_PARTS;
  return spec
    .split('+')
    .map((part) => table[part] ?? part.toUpperCase())
    .join('+');
}
