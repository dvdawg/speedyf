// @vitest-environment happy-dom
/**
 * Where a project's notepad lives: in the detail column, while the project
 * is the scope and nothing is selected. An item selected shows its details
 * under a quiet line back to the note; that line, Escape, or a click on
 * empty space in the list lets it go. Other scopes keep the details column
 * as it was, the middle column switches only List and Graph, the column
 * resizes by its left edge and remembers it, and rows dragged onto the note
 * become citations. The footer keeps New project on its own line.
 */
import { cleanup, fireEvent, render, screen, within } from '@solidjs/testing-library';
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

const JARZYNSKI = 'Nonequilibrium equality for free energy differences';
const CROOKS = 'Entropy production fluctuation theorem';

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
const middle = (root: HTMLElement) => root.querySelector('.projects-col-papers') as HTMLElement;
const rowOf = (title: string) =>
  screen.getAllByText(title)[0]!.closest('.paper-row') as HTMLElement;

describe('the project notepad in the detail column', () => {
  it('shows for a project with nothing selected, full height, with a placeholder', async () => {
    const { container } = render(() => <ProjectsScreen />);
    const note = await within(detail(container)).findByTestId('note-editor');
    expect(note.dataset.project).toBe('p1');
    expect(note.textContent).toBe(
      'Notes for Noneq stat mech. Type @ to cite an item; drag items here.'
    );
    // The items stay listed beside it; the middle column is not the note.
    expect(within(middle(container)).queryByTestId('note-editor')).toBeNull();
    await within(middle(container)).findByText(CROOKS);
  });

  it('gives way to a selected item, under a line back to the note', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await within(detail(container)).findByTestId('note-editor');
    fireEvent.click(rowOf(CROOKS));
    const back = await within(detail(container)).findByRole('button', {
      name: 'Noneq stat mech notepad',
    });
    expect(within(detail(container)).queryByTestId('note-editor')).toBeNull();
    expect(detail(container).querySelector('.paper-detail')).not.toBeNull();
    // The back line is the column's first thing.
    expect(detail(container).querySelector('.project-pane-detail')?.firstElementChild).toBe(back);

    fireEvent.click(back);
    expect(store.state.selectedId).toBeNull();
    await within(detail(container)).findByTestId('note-editor');
  });

  it('returns to the note on Escape in the detail column', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await within(detail(container)).findByTestId('note-editor');
    fireEvent.click(rowOf(CROOKS));
    const back = await within(detail(container)).findByRole('button', {
      name: 'Noneq stat mech notepad',
    });
    fireEvent.keyDown(back, { key: 'Escape' });
    expect(store.state.selectedId).toBeNull();
    await within(detail(container)).findByTestId('note-editor');
  });

  it('returns to the note on a click on empty space in the list', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await within(middle(container)).findByText(CROOKS);
    fireEvent.click(rowOf(CROOKS));
    await within(detail(container)).findByRole('button', { name: 'Noneq stat mech notepad' });
    fireEvent.click(container.querySelector('.paper-list')!);
    expect(store.state.selectedId).toBeNull();
    await within(detail(container)).findByTestId('note-editor');
  });

  it('leaves every other scope as it was', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await within(detail(container)).findByTestId('note-editor');
    for (const scope of [{ kind: 'others' }, { kind: 'unfiled' }, { kind: 'all' }] as const) {
      store.setScope(scope);
      expect(within(detail(container)).queryByTestId('note-editor')).toBeNull();
      expect(detail(container).querySelector('.paper-detail')).not.toBeNull();
      // Each scope's rows in before the next.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    // An item selected there: its details, with no line back to a note.
    store.select('b');
    await within(detail(container)).findByText('crooks1999entropy');
    expect(container.querySelector('.project-pane-back')).toBeNull();
  });

  it('is not there while the extension is off', async () => {
    notepad.views = [];
    const { container } = render(() => <ProjectsScreen />);
    await within(middle(container)).findByText(CROOKS);
    expect(within(detail(container)).queryByTestId('note-editor')).toBeNull();
    expect(detail(container).querySelector('.paper-detail')).not.toBeNull();
  });

  it('shows behind Details on a narrow window when nothing is selected', async () => {
    narrow = true;
    const { container } = render(() => <ProjectsScreen />);
    const toggle = await screen.findByRole('button', { name: 'Details' });
    fireEvent.click(toggle);
    expect(container.querySelector('.projects-screen')?.classList).toContain('is-details-open');
    await within(detail(container)).findByTestId('note-editor');
    // No resize edge on an overlay.
    expect(container.querySelector('.projects-detail-resize')).toBeNull();
  });
});

describe('the middle column switch', () => {
  it('offers only List and Graph, in the column head', async () => {
    const { container } = render(() => <ProjectsScreen />);
    const toggle = await screen.findByRole('radiogroup', { name: 'View' });
    expect(
      within(toggle)
        .getAllByRole('radio')
        .map((r) => r.textContent)
    ).toEqual(['List', 'Graph']);
    expect(middle(container).contains(toggle)).toBe(true);
    const header = container.querySelector('.projects-head') as HTMLElement;
    expect(within(header).queryByRole('radiogroup', { name: 'View' })).toBeNull();
  });

  it('reads an extension view left over from before as the list', async () => {
    store.setView('ext:dev.speedyf.notepad/notepad');
    expect(store.state.view).toBe('list');
    render(() => <ProjectsScreen />);
    const toggle = await screen.findByRole('radiogroup', { name: 'View' });
    expect(within(toggle).getByRole('radio', { name: 'List' }).getAttribute('aria-checked')).toBe(
      'true'
    );
  });
});

describe('resizing the detail column', () => {
  it('follows the left edge and remembers the width', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await within(detail(container)).findByTestId('note-editor');
    const columns = container.querySelector('.projects-columns') as HTMLElement;
    expect(columns.style.getPropertyValue('--detail-width')).toBe('400px');
    vi.spyOn(detail(container), 'getBoundingClientRect').mockReturnValue({
      right: 1200,
    } as DOMRect);
    const edge = screen.getByRole('separator', { name: 'Resize the detail column' });
    fireEvent.pointerDown(edge, { clientX: 800, button: 0 });
    fireEvent.pointerMove(window, { clientX: 700 });
    fireEvent.pointerUp(window, { clientX: 700 });
    expect(columns.style.getPropertyValue('--detail-width')).toBe('500px');
    expect(localStorage.getItem('speedyf-projects-detail-width')).toBe('500');
    // Too narrow is held at the minimum.
    fireEvent.pointerDown(edge, { clientX: 700, button: 0 });
    fireEvent.pointerMove(window, { clientX: 1150 });
    fireEvent.pointerUp(window, { clientX: 1150 });
    expect(columns.style.getPropertyValue('--detail-width')).toBe('280px');

    // A new screen reads it back.
    localStorage.setItem('speedyf-projects-detail-width', '450');
    reloadDetailWidthForTest();
    expect(columns.style.getPropertyValue('--detail-width')).toBe('450px');
  });
});

describe('dragging rows onto the notepad', () => {
  it('inserts their cite keys where they land', async () => {
    const { container } = render(() => <ProjectsScreen />);
    await within(middle(container)).findByText(CROOKS);
    const first = rowOf(JARZYNSKI);
    const second = rowOf(CROOKS);
    fireEvent.click(first);
    fireEvent.click(second, { metaKey: true });
    // A selection would hide the note; let the item go but keep the rows.
    store.select(null);
    const note = await within(detail(container)).findByTestId('note-editor');

    vi.spyOn(document, 'elementFromPoint').mockReturnValue(note);
    fireEvent.pointerDown(second, { clientX: 10, clientY: 10, button: 0 });
    fireEvent.pointerMove(second, { clientX: 900, clientY: 120 });
    expect(document.querySelector('.drag-ghost')?.textContent).toBe('2 items');
    fireEvent.pointerUp(second, { clientX: 900, clientY: 120 });

    expect(notepad.inserts).toEqual([
      { x: 900, y: 120, text: '@jarzynski1997nonequilibrium; @crooks1999entropy' },
    ]);
    // Nothing was filed: the note is not a project row.
    expect(store.state.selectedId).toBeNull();
  });
});

describe('the projects column footer', () => {
  it('keeps New project alone, with import as an icon', async () => {
    const { container } = render(() => <ProjectsScreen />);
    const footer = (await screen.findByText(/New project/)).closest(
      '.projects-new-row'
    ) as HTMLElement;
    expect(footer.textContent?.trim()).toBe('New project');
    expect(container.textContent).not.toContain('Import bundle');
    expect(
      within(footer).getByRole('button', { name: 'Import project bundle...' }).textContent
    ).toBe('');
  });
});
