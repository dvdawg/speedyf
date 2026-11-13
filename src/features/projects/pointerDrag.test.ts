// @vitest-environment happy-dom
/**
 * The pointer-drag helper on its own: a press is cancelled so no text
 * selection starts, the drag begins only past the threshold, the body wears
 * the dragging class for exactly as long as it runs, the click that follows
 * a release is swallowed, and Escape or a lost pointer ends it cleanly.
 */
import { fireEvent } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { beginPointerDrag, DRAG_THRESHOLD } from './pointerDrag';

/** A row with a button and a text field inside, the way a project row has. */
function mountRow() {
  document.body.innerHTML = `
    <div class="row" tabindex="-1">
      <span class="name">Active matter</span>
      <button type="button" class="menu">menu</button>
      <input type="text" class="field" />
    </div>
    <div class="target" data-drop-project="p1">Noneq</div>`;
  const row = document.querySelector('.row') as HTMLElement;
  const target = document.querySelector('.target') as HTMLElement;
  const onHover = vi.fn();
  const onDrop = vi.fn();
  const under = { el: null as Element | null };
  row.addEventListener('pointerdown', (e) =>
    beginPointerDrag(e, {
      label: 'Active matter',
      targetAt: () =>
        under.el?.closest('[data-drop-project]')?.getAttribute('data-drop-project') ?? null,
      onHover,
      onDrop,
    })
  );
  return { row, target, under, onHover, onDrop };
}

const ghost = () => document.querySelector('.drag-ghost');
const dragging = () => document.body.classList.contains('is-dragging');

beforeEach(() => {
  document.body.className = '';
});

afterEach(async () => {
  // The click swallow is let go on a timer; let it run before the next test.
  await new Promise((resolve) => setTimeout(resolve, 0));
  document.body.innerHTML = '';
});

describe('beginPointerDrag', () => {
  it('cancels the press on the row, and focuses it, so no selection starts', () => {
    const { row } = mountRow();
    const prevented = !fireEvent.pointerDown(row, { clientX: 5, clientY: 5, button: 0 });
    expect(prevented).toBe(true);
    expect(document.activeElement).toBe(row);
    fireEvent.pointerUp(row, { clientX: 5, clientY: 5 });
  });

  it('leaves a press on a control inside the row alone', () => {
    const { row } = mountRow();
    const field = row.querySelector('.field')!;
    expect(fireEvent.pointerDown(field, { clientX: 5, clientY: 5, button: 0 })).toBe(true);
    const menu = row.querySelector('.menu')!;
    expect(fireEvent.pointerDown(menu, { clientX: 5, clientY: 5, button: 0 })).toBe(true);
    fireEvent.pointerMove(menu, { clientX: 50, clientY: 50 });
    expect(ghost()).toBeNull();
    expect(dragging()).toBe(false);
    fireEvent.pointerUp(menu, { clientX: 50, clientY: 50 });
  });

  it('ignores every button but the first', () => {
    const { row } = mountRow();
    expect(fireEvent.pointerDown(row, { clientX: 5, clientY: 5, button: 2 })).toBe(true);
    fireEvent.pointerMove(row, { clientX: 50, clientY: 50 });
    expect(ghost()).toBeNull();
  });

  it('wears the body class and the ghost from the threshold to the release', () => {
    const { row, target, under, onHover, onDrop } = mountRow();
    fireEvent.pointerDown(row, { clientX: 0, clientY: 0, button: 0 });
    expect(dragging()).toBe(false);
    fireEvent.pointerMove(row, { clientX: DRAG_THRESHOLD - 1, clientY: 0 });
    expect(dragging()).toBe(false);
    expect(ghost()).toBeNull();
    expect(onHover).not.toHaveBeenCalled();

    under.el = target;
    fireEvent.pointerMove(row, { clientX: DRAG_THRESHOLD, clientY: 0 });
    expect(dragging()).toBe(true);
    expect(ghost()?.textContent).toBe('Active matter');
    expect(onHover).toHaveBeenLastCalledWith('p1');

    under.el = null;
    fireEvent.pointerMove(row, { clientX: 40, clientY: 0 });
    expect(onHover).toHaveBeenLastCalledWith(null);
    under.el = target;
    fireEvent.pointerMove(row, { clientX: 60, clientY: 0 });

    fireEvent.pointerUp(row, { clientX: 60, clientY: 0 });
    expect(onDrop).toHaveBeenCalledWith('p1');
    expect(dragging()).toBe(false);
    expect(ghost()).toBeNull();
    // Cleared before the drop, so a highlight never outlives the drag.
    expect(onHover).toHaveBeenLastCalledWith(null);
  });

  it('swallows the click the browser fires after the release of a drag, and only that one', () => {
    const { row } = mountRow();
    const clicked = vi.fn();
    row.addEventListener('click', clicked);
    fireEvent.pointerDown(row, { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(row, { clientX: 20, clientY: 0 });
    fireEvent.pointerUp(row, { clientX: 20, clientY: 0 });
    fireEvent.click(row);
    expect(clicked).not.toHaveBeenCalled();
    fireEvent.click(row);
    expect(clicked).toHaveBeenCalledTimes(1);
  });

  it('lets the click through when the press never became a drag', () => {
    const { row } = mountRow();
    const clicked = vi.fn();
    row.addEventListener('click', clicked);
    fireEvent.pointerDown(row, { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(row, { clientX: 2, clientY: 2 });
    fireEvent.pointerUp(row, { clientX: 2, clientY: 2 });
    fireEvent.click(row);
    expect(clicked).toHaveBeenCalledTimes(1);
  });

  it('ends on Escape, dropping nothing, and still swallows the click of the release', () => {
    const { row, target, under, onDrop, onHover } = mountRow();
    const clicked = vi.fn();
    row.addEventListener('click', clicked);
    under.el = target;
    fireEvent.pointerDown(row, { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(row, { clientX: 20, clientY: 0 });
    expect(dragging()).toBe(true);
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(dragging()).toBe(false);
    expect(ghost()).toBeNull();
    expect(onHover).toHaveBeenLastCalledWith(null);
    fireEvent.pointerUp(row, { clientX: 20, clientY: 0 });
    fireEvent.click(row);
    expect(onDrop).not.toHaveBeenCalled();
    expect(clicked).not.toHaveBeenCalled();
  });

  it('ends when the pointer is lost, without a drop', () => {
    const { row, target, under, onDrop } = mountRow();
    under.el = target;
    fireEvent.pointerDown(row, { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(row, { clientX: 20, clientY: 0 });
    expect(dragging()).toBe(true);
    fireEvent.pointerCancel(row);
    expect(dragging()).toBe(false);
    expect(ghost()).toBeNull();
    fireEvent.pointerUp(row, { clientX: 20, clientY: 0 });
    expect(onDrop).not.toHaveBeenCalled();
  });
});
