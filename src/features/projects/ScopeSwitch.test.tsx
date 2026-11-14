// @vitest-environment happy-dom
/**
 * Switching scopes back to back redraws the rows already shown before the
 * next scope's rows arrive. Uses the notepad screen setup (mocks and
 * helpers) from NotepadLayout.test.tsx.
 */
import { cleanup, render, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ExtensionView } from '../extensions/extensionStore';

const notepad = vi.hoisted(() => ({
  views: [] as unknown[],
  inserts: [] as { x: number; y: number; text: string }[],
}));

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
  openFromDialog: vi.fn(async () => {}),
  goHome: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));
vi.mock('../home/HomeScreen', () => ({
  showHomePanel: vi.fn(),
  default: () => null,
}));
vi.mock('../extensions/extensionStore', async (original) => ({
  ...(await original<object>()),
  projectViews: () => notepad.views,
}));
// The surfaces module and the extension store import each other, so a mock
// of the store does not reach the surfaces; stand in for the one function.
vi.mock('../extensions/surfaces', async (original) => {
  const actual = await original<object>();
  const { projectsStore } = await import('./projectsStore');
  return {
    ...actual,
    projectPane: () => {
      const s = projectsStore.state;
      const view = notepad.views[0] as ExtensionView | undefined;
      if (!view || s.scope.kind !== 'project' || s.person || s.focus) return null;
      return { view, projectId: s.scope.id };
    },
  };
});
// The editor itself is NoteEditor.test.tsx's; here it is a page that takes
// drops the way the real one does.
vi.mock('../extensions/editor/NoteEditor', async () => {
  const { onCleanup, onMount } = await import('solid-js');
  const { registerNoteDrop } = await import('../extensions/editor/noteDrop');
  return {
    default: (props: { projectId: string; placeholder?: string }) => {
      let el!: HTMLDivElement;
      onMount(() =>
        onCleanup(
          registerNoteDrop(el, {
            insertAt: (x, y, text) => notepad.inserts.push({ x, y, text }),
          })
        )
      );
      return (
        <div ref={el} data-testid="note-editor" data-project={props.projectId}>
          {props.placeholder}
        </div>
      );
    },
  };
});

const NOTEPAD_VIEW: ExtensionView = {
  extensionId: 'dev.speedyf.notepad',
  id: 'dev.speedyf.notepad/notepad',
  localId: 'notepad',
  title: 'Notepad',
  editor: { file: 'notepad.md', command: 'editor' },
};

const { default: ProjectsScreen } = await import('./ProjectsScreen');
const { projectsStore: store } = await import('./projectsStore');
const { memoryStorage, resetFixture } = await import('./testFixtures');
const { reloadDetailWidthForTest } = await import('./detailWidth');

let narrow = false;

beforeEach(async () => {
  narrow = false;
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (query: string) =>
      ({
        matches: narrow,
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }) as unknown as MediaQueryList
  );
  vi.stubGlobal('localStorage', memoryStorage());
  reloadDetailWidthForTest();
  notepad.views = [NOTEPAD_VIEW];
  notepad.inserts = [];
  resetFixture();
  store.setScope({ kind: 'project', id: 'p1' });
  store.leaveFocus();
  store.leavePerson();
  store.select(null);
  store.selectGhost(null);
  store.setView('list');
  await store.initialize();
});

afterEach(async () => {
  cleanup();
  store.setView('list');
  // The click swallowed after a drag's release is let go on a timer.
  await new Promise((resolve) => setTimeout(resolve, 0));
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const detail = (root: HTMLElement) => root.querySelector('.projects-col-detail') as HTMLElement;

describe('switching scopes back to back', () => {
  // The rows already drawn turn from items into group headers (and back)
  // before the next scope's rows arrive. A row's old contents could read the
  // Match accessor once more on the way out, which Solid refuses.
  for (const seq of [
    ['others', 'unfiled', 'all'],
    ['all', 'others'],
  ] as const) {
    it(`goes ${seq.join(' > ')} with an item selected and throws nothing`, async () => {
      const { container } = render(() => <ProjectsScreen />);
      await within(detail(container)).findByTestId('note-editor');
      for (const kind of seq) store.setScope({ kind });
      store.select('b');
      await new Promise((resolve) => setTimeout(resolve, 50));
      for (const kind of seq) {
        store.setScope({ kind });
        await Promise.resolve();
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
      expect(container.querySelector('.paper-list')).not.toBeNull();
    });
  }
});
