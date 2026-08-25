/** However an extension chooses to show itself.
 *
 * An extension may ship a `png` or `svg`, which is rendered in an `<img>` —
 * deliberately, because an `<img>` is the safe way to display an SVG nobody has
 * reviewed. Loaded that way it is a passive image: no scripts, no event
 * handlers, no reach into the page. Inlining the markup instead would run
 * whatever the author put in it.
 *
 * Without one, the generic extension mark carries the extension's initials in
 * its corner, so a shelf of extensions that all declined to ship an icon is
 * still one you can read.
 */
import { Show, createSignal } from 'solid-js';
import { IconExtension } from '../../components/icons';

/** Up to two initials for a name.
 *
 * Words first — "Page Audit" is `PA`, which reads better than `Pa` — falling
 * back to the first two characters of a single-word name. Exported for its
 * tests: this is the label someone identifies a panel by, and getting `#1` or
 * an empty string on screen is worse than it looks.
 */
export function initialsFor(name: string): string {
  const words = name
    .split(/[\s\-_/]+/)
    .map((word) => word.trim())
    .filter((word) => /^[\p{L}\p{N}]/u.test(word));

  if (words.length >= 2) {
    return (words[0]![0]! + words[1]![0]!).toUpperCase();
  }
  const single = words[0] ?? name.trim();
  return single.slice(0, 2).toUpperCase() || '?';
}

/** Where an extension's own icon is served from. */
export function extensionIconUrl(iconPath: string): string {
  const isWindows = typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent ?? '');
  const base = isWindows ? 'http://speedyf-ext.localhost' : 'speedyf-ext://localhost';
  return `${base}${iconPath}`;
}

export default function ExtensionIcon(props: { name: string; iconPath?: string | null }) {
  // A declared icon that fails to load falls back rather than leaving a broken
  // image: the file may have been removed since the manifest was written.
  const [failed, setFailed] = createSignal(false);

  return (
    <Show
      when={props.iconPath && !failed()}
      fallback={
        <span class="ext-icon" aria-hidden="true">
          <IconExtension />
          <span class="ext-icon-initials">{initialsFor(props.name)}</span>
        </span>
      }
    >
      <img
        class="ext-icon-image"
        src={extensionIconUrl(props.iconPath!)}
        alt=""
        draggable={false}
        onError={() => setFailed(true)}
      />
    </Show>
  );
}
