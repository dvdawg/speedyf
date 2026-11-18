// @vitest-environment happy-dom
/**
 * The toolbar row folds what does not fit into a trailing overflow button,
 * whose menu is reachable and leavable by keyboard alone.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import IconButton from './IconButton';
import OverflowRow, { type ToolbarGroup } from './OverflowRow';

const clicked: string[] = [];
const button = (label: string) => (
  <IconButton label={label} onClick={() => clicked.push(label)}>
    <svg />
  </IconButton>
);
const groups: ToolbarGroup[] = [
  { id: 'a', label: 'A', priority: Infinity, render: () => button('Alpha') },
  { id: 'b', label: 'B', priority: 1, render: () => button('Bravo (Cmd+B)') },
  {
    id: 'c',
    label: 'C',
    priority: 2,
    render: () => (
      <>
        {button('Charlie')}
        {button('Delta')}
      </>
    ),
  },
];

let rowWidth = 1000;
beforeEach(() => {
  clicked.length = 0;
  rowWidth = 1000;
  // happy-dom lays nothing out: every group is 100 wide, the row as set.
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (
    this: HTMLElement
  ) {
    return this.classList.contains('toolbar-row') ? rowWidth : 0;
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(
    () => ({ width: 100, height: 28, top: 0, left: 0, right: 100, bottom: 28 }) as DOMRect
  );
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const labels = () => screen.queryAllByRole('button').map((b) => b.getAttribute('aria-label') ?? '');

it('shows every group and no overflow button while they fit', () => {
  render(() => <OverflowRow groups={groups} trailingId="c" />);
  expect(labels()).toEqual(['Alpha', 'Bravo (Cmd+B)', 'Charlie', 'Delta']);
  expect(screen.queryByRole('button', { name: 'More tools' })).toBeNull();
});

it('folds the least important group behind a trailing overflow button', () => {
  rowWidth = 250;
  render(() => <OverflowRow groups={groups} trailingId="c" />);
  expect(labels()).toEqual(['Alpha', 'Charlie', 'Delta', 'More tools']);
});

it('opens the menu from the keyboard and walks it with the arrows', async () => {
  rowWidth = 150;
  render(() => <OverflowRow groups={groups} trailingId="c" />);
  const more = screen.getByRole('button', { name: 'More tools' });
  expect(more.getAttribute('aria-expanded')).toBe('false');
  more.focus();
  fireEvent.keyDown(more, { key: 'ArrowDown' });
  await Promise.resolve();

  const menu = screen.getByRole('menu', { name: 'More tools' });
  expect(more.getAttribute('aria-expanded')).toBe('true');
  const items = screen.getAllByRole('menuitem');
  expect(items.map((i) => i.textContent)).toEqual(['BravoCmd+B', 'Charlie', 'Delta']);
  expect(document.activeElement).toBe(items[0]);

  fireEvent.keyDown(menu, { key: 'ArrowDown' });
  expect(document.activeElement).toBe(items[1]);
  fireEvent.keyDown(menu, { key: 'ArrowUp' });
  fireEvent.keyDown(menu, { key: 'ArrowUp' });
  expect(document.activeElement).toBe(items[2]);
  fireEvent.keyDown(menu, { key: 'Home' });
  expect(document.activeElement).toBe(items[0]);

  fireEvent.keyDown(menu, { key: 'Escape' });
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(more);
});

it('runs a chosen row and closes, handing focus back to the button', async () => {
  rowWidth = 150;
  render(() => <OverflowRow groups={groups} trailingId="c" />);
  const more = screen.getByRole('button', { name: 'More tools' });
  fireEvent.click(more);
  await Promise.resolve();
  fireEvent.click(screen.getByRole('menuitem', { name: 'Charlie' }));
  expect(clicked).toEqual(['Charlie']);
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.activeElement).toBe(more);
});
