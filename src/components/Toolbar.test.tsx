// @vitest-environment happy-dom
/**
 * Home and Projects show no document, so the reader controls would all sit
 * there disabled. Only Open is shown on them; a document tab gets the lot.
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

const buttons = () => screen.getAllByRole('button').map((b) => b.getAttribute('aria-label'));

it('shows only Open on the launch screens', () => {
  render(() => <Toolbar />);
  expect(buttons()).toHaveLength(1);
  expect(buttons()[0]).toMatch(/^Open PDF/);
  expect(screen.queryByLabelText('Current page number')).toBeNull();
  expect(screen.getByRole('toolbar', { name: 'Main toolbar' })).toBeTruthy();
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
  expect(screen.getByLabelText('Current page number')).toBeTruthy();

  // Back to Home: the reader controls go, Open stays.
  setActiveId(null);
  expect(buttons()).toHaveLength(1);
});
