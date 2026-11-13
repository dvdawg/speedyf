// @vitest-environment happy-dom
/**
 * Authors as people: an author the engine knows on other items is a quiet
 * link in the detail column, it lists the person's items in the middle
 * column under "Items by ..." with the other forms of the name, and Escape
 * or the close button goes back. An identifier citing papers supplied says
 * how many vouched, as other fields do.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Paper } from '../../types/engine';

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

const { default: PaperDetail } = await import('./PaperDetail');
const { default: PaperList } = await import('./PaperList');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { paperFor } = await import('./testFixtures');

const TITLE = 'Nonequilibrium equality for free energy differences';
const CROOKS = 'Entropy production fluctuation theorem';

const withPerson = (): Paper => ({
  ...paperFor('a'),
  vouched: { doi: 3 },
  doiSource: 'citations',
  authorPersons: ['p-jarzynski'],
});

beforeEach(async () => {
  projectsStore.setScope({ kind: 'all' });
  projectsStore.leavePerson();
  await projectsStore.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.mocked(engine.libraryPaper).mockImplementation(async (id: string) => paperFor(id));
});

describe('authors as people', () => {
  it('makes a known author a link that lists their items', async () => {
    vi.mocked(engine.libraryPaper).mockResolvedValue(withPerson());
    projectsStore.select('a');
    await vi.waitFor(() => expect(projectsStore.state.selected?.id).toBe('a'));
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    const link = screen.getByRole('button', { name: 'Jarzynski, C.' });
    expect(link.classList.contains('author-link')).toBe(true);
    // The field still edits, from its own button.
    expect(screen.getByRole('button', { name: 'Edit Authors' })).toBeTruthy();
    fireEvent.click(link);
    expect(engine.libraryPerson).toHaveBeenCalledWith('p-jarzynski');
    await waitFor(() => expect(projectsStore.state.person?.name).toBe('Christopher Jarzynski'));
  });

  it('leaves an author with no other items as plain text', async () => {
    // As the engine sends it: always present, so a refresh clears it.
    vi.mocked(engine.libraryPaper).mockResolvedValue({ ...paperFor('a'), authorPersons: [null] });
    projectsStore.select('a');
    await vi.waitFor(() => expect(projectsStore.state.selected?.authorPersons).toEqual([null]));
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    expect(document.querySelector('.author-link')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Edit Authors' })).toBeNull();
  });

  it('says how many citing papers gave a DOI', async () => {
    vi.mocked(engine.libraryPaper).mockResolvedValue(withPerson());
    projectsStore.select('a');
    await vi.waitFor(() => expect(projectsStore.state.selected?.doiSource).toBe('citations'));
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    const field = screen
      .getByText('DOI', { selector: '.paper-field-label' })
      .closest('.paper-field');
    const tags = Array.from(field?.querySelectorAll('.paper-field-source') ?? []).map(
      (el) => el.textContent
    );
    expect(tags).toEqual(['from 3 citing papers']);
  });

  it('lists the person in place of the scope, with the other forms, and Escape returns', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(CROOKS);
    await projectsStore.showPerson('p-jarzynski');
    expect(await screen.findByText('Christopher Jarzynski')).toBeTruthy();
    expect(screen.getByText('also as C. Jarzynski')).toBeTruthy();
    // Flat, and only their items.
    expect(container.querySelector('.paper-group-name')).toBeNull();
    expect(screen.getAllByText(TITLE)).toHaveLength(1);
    expect(screen.queryByText(CROOKS)).toBeNull();

    fireEvent.keyDown(container.querySelector('.paper-list')!, { key: 'Escape' });
    expect(projectsStore.state.person).toBeNull();
    expect(screen.queryByText('also as C. Jarzynski')).toBeNull();
    expect(await screen.findByText(CROOKS)).toBeTruthy();
  });

  it('goes back with the close button, and a scope change leaves the person', async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(CROOKS);
    await projectsStore.showPerson('p-jarzynski');
    fireEvent.click(await screen.findByRole('button', { name: 'Leave the person' }));
    expect(projectsStore.state.person).toBeNull();

    await projectsStore.showPerson('p-jarzynski');
    expect(projectsStore.state.person).not.toBeNull();
    projectsStore.setScope({ kind: 'project', id: 'p1' });
    expect(projectsStore.state.person).toBeNull();
  });
});
