// @vitest-environment happy-dom
/**
 * An extension's view of a project (the notepad) reads as a page: the list's
 * head, with its BibTeX and lookup actions, steps aside, and the view switch
 * moves up into the screen header. List and Graph keep the switch in the
 * column head.
 */
import { cleanup, render, screen, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
  openFromDialog: vi.fn(async () => {}),
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
  projectViews: () => [
    {
      extensionId: 'dev.speedyf.notepad',
      id: 'dev.speedyf.notepad/notepad',
      localId: 'notepad',
      title: 'Notepad',
      editor: { file: 'notepad.md', command: 'edit' },
    },
  ],
}));
// The surfaces module and the extension store import each other, so a mock
// of the store does not reach the surfaces; stand in for the one function.
vi.mock('../extensions/surfaces', async (original) => {
  const actual = await original<object>();
  const { projectsStore } = await import('./projectsStore');
  return {
    ...actual,
    activeProjectView: () =>
      projectsStore.state.view === 'ext:dev.speedyf.notepad/notepad' &&
      projectsStore.state.scope.kind === 'project'
        ? {
            view: {
              extensionId: 'dev.speedyf.notepad',
              id: 'dev.speedyf.notepad/notepad',
              localId: 'notepad',
              title: 'Notepad',
              editor: { file: 'notepad.md', command: 'edit' },
            },
            projectId: projectsStore.state.scope.id,
          }
        : null,
  };
});
vi.mock('../extensions/editor/NoteEditor', () => ({
  default: () => <div data-testid="note-editor" />,
}));

const { default: ProjectsScreen } = await import('./ProjectsScreen');
const { projectsStore: store } = await import('./projectsStore');
const { memoryStorage, resetFixture } = await import('./testFixtures');

beforeEach(async () => {
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (query: string) =>
      ({
        matches: false,
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }) as unknown as MediaQueryList
  );
  vi.stubGlobal('localStorage', memoryStorage());
  resetFixture();
  store.setScope({ kind: 'project', id: 'p1' });
  store.leaveFocus();
  store.select(null);
  store.setView('list');
  await store.initialize();
});

afterEach(() => {
  cleanup();
  store.setView('list');
  vi.unstubAllGlobals();
});

describe('the notepad view of a project', () => {
  it('hides the list head and puts the view switch in the screen header', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await screen.findByRole('radiogroup', { name: 'View' });
    const header = container.querySelector('.projects-head') as HTMLElement;
    expect(container.querySelector('.paper-list-head')).not.toBeNull();
    expect(within(header).queryByRole('radiogroup', { name: 'View' })).toBeNull();

    store.setView('ext:dev.speedyf.notepad/notepad');
    await screen.findByTestId('note-editor');
    expect(container.querySelector('.paper-list-head')).toBeNull();
    expect(container.querySelector('.paper-list-bib')).toBeNull();
    const toggle = within(header).getByRole('radiogroup', { name: 'View' });
    expect(
      within(toggle).getByRole('radio', { name: 'Notepad' }).getAttribute('aria-checked')
    ).toBe('true');
    expect(screen.getAllByRole('radiogroup', { name: 'View' })).toHaveLength(1);
  });
});
