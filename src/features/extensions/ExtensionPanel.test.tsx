// @vitest-environment happy-dom
/**
 * The first component test in this repo.
 *
 * `parseUi` is already covered as pure logic, but that only proves a
 * description is *readable*. It says nothing about what reaches the screen, and
 * this component is where an extension's text becomes DOM — the one place
 * third-party content is rendered. What matters here is the wiring the pure
 * tests cannot see: that a row with no action is inert, that an action fires
 * with the right arguments, and that a keyboard user can reach one.
 */
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';

/** The open document. */
const [openDocId, setOpenDocId] = createSignal(1);

/** What the store currently holds for this panel: the rows, and which document
 * they describe. Panel content is global and outlives any one panel component,
 * which is the whole reason staleness has to be detectable. */
let ui: unknown = undefined;
let shownDoc: number | null | undefined = undefined;

const openTab = {
  id: 'tab-1',
  documentStore: {
    state: {
      get docId() {
        return openDocId();
      },
    },
  },
};

vi.mock('../../stores/tabsStore', () => ({ activeTab: () => openTab }));

const refreshPanel = vi.fn(async () => {});
const runPanelAction = vi.fn(async () => {});

vi.mock('./extensionStore', () => ({
  panelUi: () => ui,
  panelDocId: () => shownDoc,
  refreshPanel: (...args: unknown[]) => refreshPanel(...(args as [])),
  runPanelAction: (...args: unknown[]) => runPanelAction(...(args as [])),
}));

const { default: ExtensionPanelView } = await import('./ExtensionPanel');

const panel = {
  id: 'com.example.thing/refs',
  localId: 'refs',
  title: 'References',
  extensionId: 'com.example.thing',
};

beforeEach(() => {
  ui = undefined;
  shownDoc = undefined;
  setOpenDocId(1);
  refreshPanel.mockClear();
  runPanelAction.mockClear();
});

/** Panel content as it would be after a successful render of document `docId`. */
function rendered(docId: number) {
  ui = { sections: [{ rows: [{ id: 'a', title: 'A Paper' }] }] };
  shownDoc = docId;
}

afterEach(cleanup);

describe('ExtensionPanelView', () => {
  it('asks the extension to render when it has never rendered', () => {
    render(() => <ExtensionPanelView panel={panel} />);
    expect(refreshPanel).toHaveBeenCalledWith(panel.id);
  });

  it('shows a loading note until the extension answers', () => {
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    expect(getByText('Loading…')).toBeTruthy();
  });

  it('draws rows the extension described', () => {
    ui = {
      sections: [
        {
          title: 'In your library',
          rows: [{ id: 'a', title: 'A Paper', subtitle: 'cited 3×', badge: 'in library' }],
        },
      ],
    };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    expect(getByText('A Paper')).toBeTruthy();
    expect(getByText('cited 3×')).toBeTruthy();
    expect(getByText('in library')).toBeTruthy();
    expect(getByText('In your library')).toBeTruthy();
  });

  it('shows the extensions own empty message rather than a blank box', () => {
    ui = { sections: [], empty: 'No references found' };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    expect(getByText('No references found')).toBeTruthy();
  });

  it('shows an error the extension reported', () => {
    ui = { sections: [], error: 'Could not read the document' };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    expect(getByText('Could not read the document')).toBeTruthy();
  });
});

describe('row actions', () => {
  it('runs the action with the extension id, command and value', () => {
    ui = {
      sections: [
        {
          rows: [{ id: 'a', title: 'A Paper', action: { command: 'open', value: '1706.03762' } }],
        },
      ],
    };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    fireEvent.click(getByText('A Paper'));
    expect(runPanelAction).toHaveBeenCalledWith('com.example.thing', 'open', '1706.03762');
  });

  /** A row that does nothing must not look or behave like a control. */
  it('leaves a row with no action inert and out of the tab order', () => {
    ui = { sections: [{ rows: [{ id: 'a', title: 'Just text' }] }] };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    const row = getByText('Just text').closest('.ext-panel-row')!;

    expect(row.getAttribute('role')).toBe(null);
    expect(row.getAttribute('tabindex')).toBe(null);
    fireEvent.click(row);
    expect(runPanelAction).not.toHaveBeenCalled();
  });

  it('makes an actionable row reachable and operable by keyboard', () => {
    ui = { sections: [{ rows: [{ id: 'a', title: 'A Paper', action: { command: 'open' } }] }] };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    const row = getByText('A Paper').closest('.ext-panel-row')!;

    expect(row.getAttribute('role')).toBe('button');
    expect(row.getAttribute('tabindex')).toBe('0');

    fireEvent.keyDown(row, { key: 'Enter' });
    expect(runPanelAction).toHaveBeenCalledWith('com.example.thing', 'open', undefined);

    fireEvent.keyDown(row, { key: ' ' });
    expect(runPanelAction).toHaveBeenCalledTimes(2);
  });

  it('ignores keys that are not activation keys', () => {
    ui = { sections: [{ rows: [{ id: 'a', title: 'A Paper', action: { command: 'open' } }] }] };
    const { getByText } = render(() => <ExtensionPanelView panel={panel} />);
    fireEvent.keyDown(getByText('A Paper').closest('.ext-panel-row')!, { key: 'a' });
    expect(runPanelAction).not.toHaveBeenCalled();
  });
});

describe('refresh', () => {
  it('re-asks the extension when refresh is pressed', async () => {
    rendered(1);
    const { getByRole } = render(() => <ExtensionPanelView panel={panel} />);
    refreshPanel.mockClear();

    // Reachable by its accessible name, not its text: it is an icon button, so
    // the label is all a screen reader or a keyboard user has.
    fireEvent.click(getByRole('button', { name: 'Refresh' }));
    await vi.waitFor(() => expect(refreshPanel).toHaveBeenCalledWith(panel.id));
  });

  it('renders when nothing has been drawn yet', async () => {
    render(() => <ExtensionPanelView panel={panel} />);
    await vi.waitFor(() => expect(refreshPanel).toHaveBeenCalledWith(panel.id));
  });

  it('re-renders when another document becomes active', async () => {
    rendered(1);
    render(() => <ExtensionPanelView panel={panel} />);
    refreshPanel.mockClear();

    setOpenDocId(2);

    // Otherwise the panel keeps showing the previous paper's rows, which look
    // current and are not.
    await vi.waitFor(() => expect(refreshPanel).toHaveBeenCalledWith(panel.id));
  });

  /** The case every earlier version of this test missed.
   *
   * Each tab mounts its own sidebar, so switching documents disposes this
   * component and mounts a fresh one. The new panel finds the store already
   * populated — with the previous document's rows — and any test based on
   * mount timing or on reacting to a change concludes there is nothing to do.
   * The panel showed the wrong paper's references until something else forced
   * a render. */
  it('renders on mount when the stored rows describe another document', async () => {
    rendered(1);
    render(() => <ExtensionPanelView panel={panel} />);
    refreshPanel.mockClear();

    // The tab switch: this component goes away, the content does not.
    cleanup();
    setOpenDocId(2);
    render(() => <ExtensionPanelView panel={panel} />);

    await vi.waitFor(() => expect(refreshPanel).toHaveBeenCalledWith(panel.id));
  });

  it('does not re-render when the rows already describe the open document', async () => {
    rendered(1);
    render(() => <ExtensionPanelView panel={panel} />);
    refreshPanel.mockClear();

    setOpenDocId(1);
    await Promise.resolve();

    expect(refreshPanel).not.toHaveBeenCalled();
  });
});
