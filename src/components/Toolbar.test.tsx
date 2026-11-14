// @vitest-environment happy-dom
/**
 * Home and Projects show no document, so the reader controls would all sit
 * there disabled and Open alone would be an empty bar: they have no toolbar
 * row at all (Open stays on Cmd+O, Home's button and the palette). A
 * document tab gets the lot.
 */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, expect, it } from 'vitest';
import Toolbar from './Toolbar';
import { addTab, createTab, removeTab, setActiveId, tabsStore } from '../stores/tabsStore';

afterEach(() => {
  cleanup();
  for (const tab of [...tabsStore.state.tabs]) {
    tab.citationStore.dispose();
    removeTab(tab.id);
  }
  setActiveId(null);
});

const buttons = () => screen.queryAllByRole('button').map((b) => b.getAttribute('aria-label'));

it('renders no toolbar row on the launch screens', () => {
  const { container } = render(() => <Toolbar />);
  expect(screen.queryByRole('toolbar')).toBeNull();
  expect(buttons()).toHaveLength(0);
  expect(container.querySelector('.toolbar')).toBeNull();
});

it('shows every reader control while a document tab is showing', () => {
  const tab = createTab();
  addTab(tab);
  setActiveId(tab.id);
  render(() => <Toolbar />);
  const labels = buttons();
  expect(labels).toContain('Toggle sidebar');
  expect(labels.some((l) => l?.startsWith('Save ('))).toBe(true);
  expect(labels).toContain('Highlight text');
  expect(labels.some((l) => l?.startsWith('Open PDF'))).toBe(true);
  expect(screen.getByLabelText('Current page number')).toBeTruthy();

  // Back to Home: the whole row goes.
  setActiveId(null);
  expect(screen.queryByRole('toolbar')).toBeNull();
});
