/** A sheet: the one shape every confirm and choice dialog takes. It hangs
 * from the top centre of the window, under the title bar row on macOS,
 * over content dimmed beneath it, and fades in (never with reduced
 * motion). Keys follow sheetKeys.ts; focus moves in on open, stays inside,
 * and goes back where it was on close. */
import { onCleanup, onMount, type JSX } from 'solid-js';
import { handleSheetKey, initialFocus } from './sheetKeys';

interface Props {
  label: string;
  /** Escape, and whatever else means "not now". */
  onCancel: () => void;
  /** Extra classes for the sheet panel (its width and inner layout). */
  class?: string;
  children: JSX.Element;
}

export default function Sheet(props: Props) {
  let sheet!: HTMLDivElement;
  onMount(() => {
    const restore = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    queueMicrotask(() => {
      if (sheet.isConnected && !sheet.contains(document.activeElement)) initialFocus(sheet).focus();
    });
    onCleanup(() => {
      if (restore?.isConnected) queueMicrotask(() => restore.focus());
    });
  });
  return (
    <div
      class="modal-backdrop sheet-backdrop"
      onKeyDown={(e) => handleSheetKey(e, sheet, props.onCancel)}
    >
      <div
        ref={sheet}
        class={`modal sheet${props.class ? ` ${props.class}` : ''}`}
        role="dialog"
        aria-modal="true"
        aria-label={props.label}
        tabindex="-1"
      >
        {props.children}
      </div>
    </div>
  );
}
