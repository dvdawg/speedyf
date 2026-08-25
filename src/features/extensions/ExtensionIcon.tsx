/** However an extension chooses to show itself.
 *
 * An extension may ship a `png`, `jpg` or `svg`, and it is never inlined: doing
 * that would run whatever the author put in the file. It is drawn as a CSS mask
 * instead — the image supplies the shape, `currentColor` supplies the colour,
 * and a mask executes nothing.
 *
 * That also fixes what an `<img>` could not. These icons sit in the sidebar
 * rail beside the built-in ones, which are stroked in `currentColor` and follow
 * the theme. An `<img>` renders the file's own colours, and `currentColor`
 * inside one resolves to black — so a correctly written icon still landed as a
 * black smudge next to fifteen grey ones. Masking makes an extension's icon
 * match the rail by construction, in both themes and with the page palette
 * inverted.
 *
 * The icon arrives as a `data:` URI rather than a `speedyf-ext://` URL, and
 * that is not incidental: a CSS mask is subject to origin restrictions an
 * `<img>` is not, and a cross-origin one paints an empty square in silence.
 * `icon_data_uri` on the Rust side records the measurement.
 *
 * The cost is that icons are monochrome. That is the same trade VS Code makes
 * for its activity bar, and for a 16px glyph in a rail of 16px glyphs it is the
 * right one.
 *
 * Without an icon, the generic extension mark carries the extension's initials
 * in its corner, so a shelf of extensions that all declined to ship one is
 * still one you can read.
 */
import { Show } from 'solid-js';
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

/** Whether a value is safe to drop into a CSS `url()` unquoted-by-us.
 *
 * The backend builds these, so this is a guard rather than a sanitiser: it
 * proves the string is one of ours before it reaches a stylesheet, instead of
 * trusting that it is. Base64 has no character that can end a `url()` token,
 * so a value matching this cannot carry a second declaration with it.
 */
export function isIconDataUri(value: string): boolean {
  return /^data:image\/(svg\+xml|png|jpeg);base64,[A-Za-z0-9+/]+=*$/.test(value);
}

export default function ExtensionIcon(props: { name: string; iconPath?: string | null }) {
  const usable = () => {
    const value = props.iconPath;
    return value && isIconDataUri(value) ? value : null;
  };

  return (
    <Show
      when={usable()}
      fallback={
        <span class="ext-icon" aria-hidden="true">
          <IconExtension />
          <span class="ext-icon-initials">{initialsFor(props.name)}</span>
        </span>
      }
    >
      <span
        class="ext-icon-image"
        aria-hidden="true"
        style={{
          // Both spellings: the unprefixed property is the standard one, and
          // the WebKit view this ships in still wants its own.
          '-webkit-mask-image': `url("${usable()}")`,
          'mask-image': `url("${usable()}")`,
        }}
      />
    </Show>
  );
}
