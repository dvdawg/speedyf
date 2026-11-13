// @vitest-environment happy-dom
/**
 * Where a file dropped from the desktop lands, read off the DOM under the
 * point Tauri reports; and what the overlay says about it.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});

const { dropHover, dropOverlayText, projectsDropTarget, setDropHover } = await import('./dnd');
const { projectsStore } = await import('./projectsStore');

describe('projectsDropTarget', () => {
  let under: ReturnType<typeof vi.spyOn>;
  /** The screen's skeleton: the parts the hit-test reads, nothing else. */
  const part = (selector: string) => document.querySelector(selector) as Element;

  beforeEach(async () => {
    await projectsStore.initialize();
    document.body.innerHTML = `
      <div class="status-bar">status</div>
      <div class="projects-screen">
        <div data-drop-project="p1"><span class="name">Noneq stat mech</span></div>
        <div data-drop-project="gone"><span class="name">Deleted</span></div>
        <div class="paper-list"><div class="paper-row">a row</div></div>
      </div>`;
    under = vi.spyOn(document, 'elementFromPoint').mockReturnValue(null);
  });
  afterEach(() => {
    under.mockRestore();
    document.body.innerHTML = '';
  });

  it('is null with nothing, or no Projects screen, under the point', () => {
    expect(projectsDropTarget(5, 5)).toBeNull();
    under.mockReturnValue(part('.status-bar'));
    expect(projectsDropTarget(5, 5)).toBeNull();
    // The screen unmounted: the chrome is all there is.
    document.body.innerHTML = '<div class="home-screen"><div class="paper-row">x</div></div>';
    under.mockReturnValue(part('.paper-row'));
    expect(projectsDropTarget(5, 5)).toBeNull();
  });

  it('names the project whose row is under the point', () => {
    under.mockReturnValue(part('[data-drop-project="p1"] .name'));
    expect(projectsDropTarget(5, 5)).toEqual({
      kind: 'project',
      id: 'p1',
      name: 'Noneq stat mech',
    });
  });

  it('is the screen anywhere else on it, and for a row of a project the store lost', () => {
    under.mockReturnValue(part('.paper-row'));
    expect(projectsDropTarget(5, 5)).toEqual({ kind: 'screen' });
    under.mockReturnValue(part('[data-drop-project="gone"]'));
    expect(projectsDropTarget(5, 5)).toEqual({ kind: 'screen' });
  });
});

describe('dropOverlayText', () => {
  it('says what the drop would do where it is', () => {
    expect(dropOverlayText(null)).toBe('Drop PDF to open');
    expect(dropOverlayText({ kind: 'screen' })).toBe('Drop PDF to add to the library');
    expect(dropOverlayText({ kind: 'project', id: 'p1', name: 'Active matter' })).toBe(
      'Drop PDF to add to Active matter'
    );
  });
});

describe('setDropHover', () => {
  it('keeps the same target as the same value, so rows are not re-evaluated per move', () => {
    setDropHover({ kind: 'into', id: 'p1' });
    const first = dropHover();
    setDropHover({ kind: 'into', id: 'p1' });
    expect(dropHover()).toBe(first);
    setDropHover({ kind: 'slot', id: 'p1', after: true });
    expect(dropHover()).toEqual({ kind: 'slot', id: 'p1', after: true });
    setDropHover({ kind: 'slot', id: 'p1', after: false });
    expect(dropHover()).toEqual({ kind: 'slot', id: 'p1', after: false });
    setDropHover(null);
    expect(dropHover()).toBeNull();
  });
});
