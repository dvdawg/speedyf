/** Inline 16px stroke icons (no icon-font dependency, theme-aware via currentColor). */
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

export const IconOpen = () =>
  base(
    <path d="M1.5 4.5a1 1 0 0 1 1-1h3l1.5 2h6.5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-8z" />
  );
export const IconHome = () =>
  base(
    <>
      <path d="M2 7.5 8 2l6 5.5" />
      <path d="M3.5 6.5V13.5a.5.5 0 0 0 .5.5h3v-4h2v4h3a.5.5 0 0 0 .5-.5V6.5" />
    </>
  );
export const IconSave = () =>
  base(
    <>
      <path d="M2.5 2.5h9l2 2v9h-11v-11z" />
      <path d="M5 2.5v3.5h5V2.5" />
      <rect x="5" y="9" width="6" height="4.5" />
    </>
  );
export const IconSaveAs = () =>
  base(
    <>
      <path d="M2.5 2.5h9l2 2v9h-11v-11z" />
      <path d="M5.5 10.5l5-5 1.5 1.5-5 5-2 .5.5-2z" />
    </>
  );
export const IconUndo = () =>
  base(
    <>
      <path d="M2.5 6.5h7a4 4 0 0 1 0 8h-2" />
      <path d="M5.5 3.5l-3 3 3 3" />
    </>
  );
export const IconRedo = () =>
  base(
    <>
      <path d="M13.5 6.5h-7a4 4 0 0 0 0 8h2" />
      <path d="M10.5 3.5l3 3-3 3" />
    </>
  );
export const IconBack = () =>
  base(
    <>
      <path d="M3 8h10.5" />
      <path d="M6.5 4.5L3 8l3.5 3.5" />
    </>
  );
export const IconPrev = () =>
  base(
    <path
      d="M8 3.5l-4.5 4.5L8 12.5M12 3.5L7.5 8l4.5 4.5"
      transform="translate(-1.5 0) scale(0.9) translate(1.5 1)"
    />
  );
export const IconChevronUp = () => base(<path d="M3.5 10l4.5-4.5L12.5 10" />);
export const IconChevronDown = () => base(<path d="M3.5 6l4.5 4.5L12.5 6" />);
export const IconZoomIn = () =>
  base(
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5L14 14M5 7h4M7 5v4" />
    </>
  );
export const IconZoomOut = () =>
  base(
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5L14 14M5 7h4" />
    </>
  );
export const IconFitPage = () =>
  base(
    <>
      <rect x="2.5" y="1.5" width="11" height="13" rx="1" />
      <path d="M5.5 5.5h5v5h-5z" />
    </>
  );
export const IconFitWidth = () =>
  base(
    <>
      <rect x="1.5" y="3.5" width="13" height="9" rx="1" />
      <path d="M4 8h8M4 8l1.5-1.5M4 8l1.5 1.5M12 8l-1.5-1.5M12 8l-1.5 1.5" />
    </>
  );
export const IconRotate = () =>
  base(
    <>
      <path d="M13.5 8a5.5 5.5 0 1 1-2-4.2" />
      <path d="M13.5 1.5v3h-3" />
    </>
  );
export const IconSearch = () =>
  base(
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5L14 14" />
    </>
  );
export const IconSidebar = () =>
  base(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M5.5 2.5v11" />
    </>
  );
export const IconPages = () =>
  base(
    <>
      <rect x="3" y="2" width="8" height="10.5" rx="0.8" />
      <path d="M5.2 5h3.6M5.2 7.3h3.6M5.2 9.6h2.4" />
    </>
  );
export const IconList = () =>
  base(
    <>
      <circle cx="2.6" cy="4" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="2.6" cy="8" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="2.6" cy="12" r="0.9" fill="currentColor" stroke="none" />
      <path d="M5.5 4h8M5.5 8h8M5.5 12h8" />
    </>
  );
export const IconSelect = () => base(<path d="M4 2l8 6-3.5.8L10 13l-2 1-1.5-4L4 12V2z" />);
export const IconHighlight = () =>
  base(
    <>
      <path d="M3 10.5l6-6 2.5 2.5-6 6H3v-2.5z" />
      <path d="M9.5 4l2.5 2.5" />
      <path d="M2 14.5h12" />
    </>
  );
export const IconInk = () => base(<path d="M2 13.5c2-4 3.5-6 5-6s1 3 2.5 3 2-2.5 4.5-5" />);
export const IconRect = () => base(<rect x="2.5" y="4.5" width="11" height="7" rx="0.5" />);
export const IconTextBox = () =>
  base(
    <>
      <rect x="1.5" y="3.5" width="13" height="9" rx="1" />
      <path d="M5 6.5h6M8 6.5v4" />
    </>
  );
export const IconNote = () =>
  base(
    <>
      <path d="M2.5 2.5h11v8h-5l-3 3v-3h-3v-8z" />
      <path d="M5 5.5h6M5 7.75h4" />
    </>
  );
export const IconEdit = () =>
  base(
    <>
      <path d="M9.5 3.5l3 3-7 7H2.5v-3l7-7z" />
      <path d="M8 5l3 3" />
    </>
  );
export const IconImage = () =>
  base(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <circle cx="5.5" cy="6" r="1.2" />
      <path d="M2.5 12l3.5-3.5 2.5 2.5 3-3 2 2" />
    </>
  );
export const IconForm = () =>
  base(
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
      <path d="M4 5.5h5M4 8h8M4 10.5h6" />
    </>
  );
export const IconPlus = () => base(<path d="M8 3v10M3 8h10" />);
export const IconTrash = () =>
  base(
    <>
      <path d="M2.5 4.5h11" />
      <path d="M5.5 4.5v-2h5v2" />
      <path d="M4 4.5l.7 9h6.6l.7-9" />
    </>
  );
export const IconDuplicate = () =>
  base(
    <>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1" />
      <path d="M10.5 5.5v-3h-8v8h3" />
    </>
  );
export const IconClose = () => base(<path d="M3.5 3.5l9 9M12.5 3.5l-9 9" />);
