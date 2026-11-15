/** The colour of each kind of comment, from the app's own tokens (their
 * light-theme values, since an exported page is white paper): proofs
 * --danger, exposition --accent, literature --warn, typos --text-dim. On
 * screen the marks use the tokens themselves (global.css, `.mk-mark`). */
import type { CommentKind } from './types';

export const KIND_INK: Record<CommentKind, string> = {
  proof: '#c62828',
  exposition: '#694fe3',
  literature: '#a15c00',
  typo: '#5c6570',
};

/** `hex` mixed into white: `amount` 0 is white, 1 the colour itself. What a
 * highlight on paper wants, so the words under it stay easy to read. */
export function tint(hex: string, amount: number): string {
  const v = Number.parseInt(hex.replace('#', ''), 16);
  const channel = (shift: number) => {
    const c = (v >> shift) & 0xff;
    return Math.round(255 - (255 - c) * amount)
      .toString(16)
      .padStart(2, '0');
  };
  return `#${channel(16)}${channel(8)}${channel(0)}`;
}

/** The highlight colour of a kind on the exported page. */
export function markColor(kind: CommentKind): string {
  return kind === 'typo' ? KIND_INK.typo : tint(KIND_INK[kind], 0.3);
}
