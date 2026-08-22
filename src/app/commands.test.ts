import { describe, expect, it, vi } from 'vitest';
import { COMMANDS, commandById, runCommand, type CommandCtx } from './commands';

/** A tab with just enough shape for `enabled` to interrogate. Anything a
 * command actually calls is a spy, so running one is observable without a
 * real document store. */
function fakeTab(over: {
  loaded?: boolean;
  pages?: number;
  selected?: { pageId: string; annotId: string } | null;
  currentPage?: number;
}) {
  const apply = vi.fn();
  const pages = Array.from({ length: over.pages ?? 3 }, (_, i) => ({
    id: `p${i}`,
    widthPt: 612,
    heightPt: 792,
  }));
  return {
    apply,
    tab: {
      id: 'tab-1',
      opening: false,
      documentStore: {
        state: {
          loaded: over.loaded ?? true,
          pages,
          selected: over.selected ?? null,
        },
        apply,
        undo: vi.fn(),
        redo: vi.fn(),
        setSelected: vi.fn(),
      },
      viewport: {
        state: { currentPage: over.currentPage ?? 0, searchOpen: false, formPanelOpen: false },
        setState: vi.fn(),
        requestScrollToPage: vi.fn(),
      },
      zoom: {
        zoomStep: vi.fn(),
        applyFit: vi.fn(),
        setZoomAnchored: vi.fn(),
      },
    } as unknown as NonNullable<CommandCtx['tab']>,
  };
}

const empty: CommandCtx = { tab: null };

describe('command registry', () => {
  it('has no duplicate ids', () => {
    const ids = COMMANDS.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('never shows a blank title', () => {
    for (const command of COMMANDS) expect(command.title.trim().length).toBeGreaterThan(0);
  });

  it('survives being asked about an empty app', () => {
    // Every enabled() runs on the home screen, where there is no tab at all.
    // A throw here would take the whole palette down with it.
    for (const command of COMMANDS) {
      expect(() => command.enabled(empty)).not.toThrow();
    }
  });

  it('disables document commands when nothing is open', () => {
    for (const id of ['file.save', 'file.print', 'edit.undo', 'page.rotateRight', 'view.fitPage']) {
      expect(commandById(id)!.enabled(empty), id).toBe(false);
    }
  });

  it('keeps the always-available commands reachable with no document', () => {
    for (const id of ['file.open', 'file.goHome', 'search.library']) {
      expect(commandById(id)!.enabled(empty), id).toBe(true);
    }
  });

  it('refuses to run a disabled command', () => {
    const ran = runCommand('page.rotateRight', empty);
    expect(ran).toBe(false);
  });

  it('reports an unknown id rather than throwing', () => {
    expect(runCommand('nope.not.a.command', empty)).toBe(false);
    expect(commandById('nope.not.a.command')).toBeUndefined();
  });
});

describe('page commands', () => {
  it('rotates the page the viewport is on, not the first one', () => {
    const { tab, apply } = fakeTab({ currentPage: 2 });
    runCommand('page.rotateRight', { tab });
    expect(apply).toHaveBeenCalledWith({ type: 'rotate', pageId: 'p2', delta: 90 });
  });

  it('rotates left as a 270 delta, so undo stays a plain inverse', () => {
    const { tab, apply } = fakeTab({});
    runCommand('page.rotateLeft', { tab });
    expect(apply).toHaveBeenCalledWith({ type: 'rotate', pageId: 'p0', delta: 270 });
  });

  it('clamps a viewport index past the end of a shortened document', () => {
    const { tab, apply } = fakeTab({ pages: 2, currentPage: 9 });
    runCommand('page.duplicate', { tab });
    expect(apply).toHaveBeenCalledWith({ type: 'duplicate', pageId: 'p1' });
  });

  it('will not delete the only page', () => {
    const { tab } = fakeTab({ pages: 1 });
    expect(commandById('page.delete')!.enabled({ tab })).toBe(false);
  });

  it('deletes a page when others remain', () => {
    const { tab, apply } = fakeTab({ pages: 2 });
    expect(runCommand('page.delete', { tab })).toBe(true);
    expect(apply).toHaveBeenCalledWith({ type: 'delete', pageId: 'p0' });
  });
});

describe('selection commands', () => {
  it('is disabled with nothing selected', () => {
    const { tab } = fakeTab({ selected: null });
    expect(commandById('edit.deleteSelection')!.enabled({ tab })).toBe(false);
  });

  it('deletes exactly the selected annotation', () => {
    const { tab, apply } = fakeTab({ selected: { pageId: 'p1', annotId: 'a7' } });
    expect(runCommand('edit.deleteSelection', { tab })).toBe(true);
    expect(apply).toHaveBeenCalledWith({ type: 'deleteAnnot', pageId: 'p1', id: 'a7' });
  });
});

describe('navigation commands', () => {
  it('stops at the last page', () => {
    const { tab } = fakeTab({ pages: 3, currentPage: 2 });
    runCommand('nav.nextPage', { tab });
    expect(tab.viewport.requestScrollToPage).toHaveBeenCalledWith(2);
  });

  it('stops at the first page', () => {
    const { tab } = fakeTab({ currentPage: 0 });
    runCommand('nav.previousPage', { tab });
    expect(tab.viewport.requestScrollToPage).toHaveBeenCalledWith(0);
  });
});
