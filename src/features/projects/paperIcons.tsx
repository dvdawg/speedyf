/** Icons for the Paper panel. Kept out of components/icons.tsx so the panel
 * and the icon set can change independently; same 16px stroke style. */
import type { JSX } from 'solid-js';

const base = (d: JSX.Element) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {d}
  </svg>
);

/** A bookmark: the paper's place in your library, as opposed to the pages,
 * outline and notes buttons beside it, which are all about its contents. */
export const IconPaper = () =>
  base(
    <>
      <path d="M4 2.5h8a.5.5 0 0 1 .5.5v10.5L8 10.5l-4.5 3V3a.5.5 0 0 1 .5-.5z" />
      <path d="M6 6h4" />
    </>
  );
