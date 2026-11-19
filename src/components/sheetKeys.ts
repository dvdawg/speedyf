/** Keyboard rules every sheet follows, as AppKit sheets do:
 *
 * - Escape cancels.
 * - Return presses the default button: the sheet's `.primary-btn` (the
 *   last one in its action row), unless it is disabled. A sheet whose action
 *   cannot be undone makes Cancel the default, so Return never commits it.
 *   Where Return already means something - a focused button, link or
 *   textarea, a text field inside a form (which submits it) - it keeps
 *   that meaning.
 * - Tab and Shift+Tab stay inside the sheet.
 *
 * Kept free of Solid so it can be tested on plain DOM. */

const FOCUSABLE = [
  'button:not(:disabled)',
  'input:not(:disabled):not([type="hidden"])',
  'select:not(:disabled)',
  'textarea:not(:disabled)',
  'a[href]',
  'summary',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

const TEXT_INPUT = /^(text|password|search|email|url|tel|number|date|time)$/;

export function sheetFocusables(sheet: HTMLElement): HTMLElement[] {
  return [...sheet.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) => !el.closest('[hidden], [inert]')
  );
}

/** The button Return presses, enabled or not. */
export function defaultButton(sheet: HTMLElement): HTMLButtonElement | null {
  const rows = sheet.querySelectorAll('.modal-actions');
  const row = rows[rows.length - 1];
  const primaries = row?.querySelectorAll<HTMLButtonElement>('.primary-btn');
  return primaries?.[primaries.length - 1] ?? null;
}

/** Where focus lands when a sheet opens: the first text field, else the
 * default button, else the first control, else the sheet itself. */
export function initialFocus(sheet: HTMLElement): HTMLElement {
  const controls = sheetFocusables(sheet);
  const field = controls.find(
    (el) =>
      el instanceof HTMLTextAreaElement ||
      (el instanceof HTMLInputElement && TEXT_INPUT.test(el.type))
  );
  const fallback = defaultButton(sheet);
  return field ?? (fallback && !fallback.disabled ? fallback : null) ?? controls[0] ?? sheet;
}

/** Whether Return already has a meaning where it was pressed. */
function returnIsTaken(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target.closest('button, a[href], textarea, select, summary')) return true;
  if (target instanceof HTMLInputElement && TEXT_INPUT.test(target.type) && target.form)
    return true;
  return false;
}

export function handleSheetKey(e: KeyboardEvent, sheet: HTMLElement, cancel: () => void) {
  if (e.key === 'Escape') {
    e.preventDefault();
    e.stopPropagation();
    cancel();
    return;
  }
  if (e.key === 'Enter') {
    if (e.isComposing || e.shiftKey || e.altKey || e.metaKey || e.ctrlKey) return;
    if (returnIsTaken(e.target)) return;
    const button = defaultButton(sheet);
    if (!button) return;
    e.preventDefault();
    e.stopPropagation();
    if (!button.disabled) button.click();
    return;
  }
  if (e.key !== 'Tab') return;
  const controls = sheetFocusables(sheet);
  const first = controls[0];
  const last = controls[controls.length - 1];
  if (!first || !last) {
    e.preventDefault();
    return;
  }
  const active = document.activeElement;
  const inside = active instanceof Node && sheet.contains(active);
  if (e.shiftKey && (active === first || !inside)) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (active === last || !inside)) {
    e.preventDefault();
    first.focus();
  }
}
