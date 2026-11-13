// @vitest-environment happy-dom
/**
 * What the citation graph adds to the detail column: a value citing papers
 * agreed on says how many vouched, an item with no year shows the year its
 * references give as "2019+", and a year older than what it cites is
 * questioned quietly.
 */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Paper } from '../../types/engine';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));

const { default: PaperDetail } = await import('./PaperDetail');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { paperFor } = await import('./testFixtures');

const TITLE = 'Nonequilibrium equality for free energy differences';

const show = async (change: (paper: Paper) => void) => {
  // As the engine sends them: always present, so a refresh clears them.
  const paper: Paper = { ...paperFor('a'), vouched: {}, yearFloor: null, yearHint: null };
  change(paper);
  vi.mocked(engine.libraryPaper).mockResolvedValue(paper);
  await projectsStore.loadDetail('a');
  render(() => <PaperDetail onMoveFocus={() => {}} />);
  await screen.findByText(TITLE);
};

/** The provenance and hints beside the field labelled `label`. */
const tags = (label: string): string[] => {
  const field = screen.getByText(label, { selector: '.paper-field-label' }).closest('.paper-field');
  return Array.from(field?.querySelectorAll('.paper-field-source') ?? []).map(
    (el) => el.textContent ?? ''
  );
};

beforeEach(async () => {
  await projectsStore.initialize();
  projectsStore.select('a');
  await vi.waitFor(() => expect(projectsStore.state.selected?.id).toBe('a'));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.mocked(engine.libraryPaper).mockImplementation(async (id: string) => paperFor(id));
});

describe('PaperDetail and the citation graph', () => {
  it('says how many citing papers vouched for a value', async () => {
    await show((paper) => {
      paper.meta.authors = { value: [{ family: 'Jarzynski', given: 'C.' }], source: 'citations' };
      paper.vouched = { authors: 3 };
    });
    expect(tags('Authors')).toEqual(['from 3 citing papers']);
  });

  it('shows the floor as the year of an item without one, from its references', async () => {
    await show((paper) => {
      paper.meta.year = null;
      paper.yearFloor = 2019;
    });
    expect(screen.getByText('2019+')).toBeTruthy();
    expect(tags('Year')).toEqual(['from its references']);
  });

  it('questions a year older than what the item cites, quietly', async () => {
    await show((paper) => {
      paper.yearHint = 'cites work from 2021; year may be wrong';
    });
    expect(screen.getByText('1997')).toBeTruthy();
    expect(tags('Year')).toEqual(['Crossref', 'cites work from 2021; year may be wrong']);
    expect(document.querySelector('.paper-badge')).toBeNull();
  });

  it('says nothing more without a floor or a hint', async () => {
    await show(() => {});
    expect(tags('Year')).toEqual(['Crossref']);
    expect(screen.queryByText(/\+$/)).toBeNull();
  });
});
