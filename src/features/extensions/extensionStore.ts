/** What is installed, what is running, and what each one is allowed to do.
 *
 * One store for the window: extensions are not per-tab. Their read APIs are
 * scoped to whichever document is *active*, so an extension never names one and
 * cannot reach a tab the user is not looking at.
 *
 * Workers are started lazily. An installed extension costs nothing until one of
 * its commands is actually run, which keeps a shelf of installed-but-unused
 * extensions off the startup path.
 */
import { createEffect, createRoot, createSignal } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { invoke } from '@tauri-apps/api/core';
import { ExtensionHost, blobWorkerFactory } from './host';
import {
  INVOKE_TIMEOUT_MS,
  NETWORK_INVOKE_TIMEOUT_MS,
  type CallMessage,
  type EditorEvent,
  type EquationTarget,
  type InvokeValue,
  type ItemLineRequest,
} from './protocol';
import { COMMANDS, runCommand } from '../../app/commands';
import { activeTab } from '../../stores/tabsStore';
import { jumpToPage } from '../viewer/navHistory';
import { reservedChords } from '../../app/shortcuts';
import { selectedGeometry, selectedText } from '../../stores/selectionStore';
import { canRunGroup, refusalReason, resolveBindings, type ExtensionBinding } from './capabilities';
import {
  API_PERMISSIONS,
  OPEN_APIS,
  apiRefusalReason,
  canCallApi,
  readCitationArg,
  readFetchArgs,
  readIdArg,
  readPageArg,
  readPlaceArg,
  readQueryArg,
  readRegionArg,
  readShowMathArgs,
  toCitations,
  toGlyphs,
  toMathTags,
  toDocumentInfo,
  toFigures,
  toLibraryHits,
  toJumps,
  toLinks,
  toPageRuns,
  toOutline,
  toPageText,
  toPaper,
  toPapers,
  toProjects,
  toResolvedCitation,
} from './api';
import { engine } from '../../lib/transport/engine';
import { pdfRectToCssRect } from '../../lib/coordinates/coords';
import { whenHolds } from './readerContext';
import { equationFor, equationToken, type EquationRef, type ItemLine } from './editorHost';
import { openApi, projectsApi } from './projectsApi';
import { captureRegion, snapshotRegion } from './regionCapture';
import { revealPlace } from '../../stores/revealStore';
import { parseUi, type UiDescription } from './ui';
import { ReferenceQueue, chooseProvider, idleWait, readerBusy } from './referenceProvider';
import { TermSession, chooseTermProvider } from './termProvider';
import { clearAllTerms, setDocumentTerms } from '../hovers/hoverStore';
import {
  openMathLoading,
  settleMath,
  showMath,
  showMathError,
  type MathAnchor,
} from './mathPopoverStore';

export interface ExtensionKeybinding {
  command: string;
  key: string;
}

export interface ExtensionSetting {
  key: string;
  title: string;
  type: 'text' | 'checkbox' | 'select';
  default?: string;
  options?: string[];
  description?: string;
}

export interface ExtensionAction {
  /** `<extension id>/<command id>` */
  id: string;
  localId: string;
  title: string;
  extensionId: string;
  /** Offered only while this holds (`documentInProject`). */
  when?: string | null;
}

/** A Markdown editor SpeedyF draws for an extension, bound to one of its
 * project files. */
export interface ExtensionEditor {
  file: string;
  /** The bare command invoked with editor events. */
  command: string;
}

/** A drawer on the right of the reader (`readerPanes`). */
export interface ExtensionPane {
  /** `<extension id>/<pane id>` */
  id: string;
  localId: string;
  title: string;
  extensionId: string;
  when: string | null;
  /** The chord that toggles it, as the manifest spells it. */
  key: string | null;
  editor: ExtensionEditor;
}

/** A view of one project in the Projects middle column (`projectViews`). */
export interface ExtensionView {
  /** `<extension id>/<view id>` */
  id: string;
  localId: string;
  title: string;
  extensionId: string;
  editor: ExtensionEditor;
}

export interface ExtensionPanel {
  /** `<extension id>/<panel id>` */
  id: string;
  localId: string;
  title: string;
  extensionId: string;
}

/** The icon an extension ships, if it ships one. */
export function extensionIconPath(extensionId: string): string | null {
  return state.extensions.find((e) => e.id === extensionId)?.iconUrl ?? null;
}

export interface ExtensionCommand {
  /** `<extension id>/<command id>`, namespaced by the Rust side. */
  id: string;
  localId: string;
  title: string;
  keywords: string[];
  /** Listed only while this holds. */
  when?: string | null;
}

export interface ExtensionManifest {
  id: string;
  name: string;
  version: string;
  permissions: string[];
}

export interface ExtensionRecord {
  id: string;
  name: string;
  version: string;
  description?: string;
  source: string;
  permissions: string[];
  enabled: boolean;
  /** True when it ships inside SpeedyF. A built-in can be switched off but not
   * removed: its files live in the app bundle and come back with every
   * update. */
  builtin: boolean;
  manifest: ExtensionManifest | null;
  commands: ExtensionCommand[];
  panels: ExtensionPanel[];
  selectionActions: ExtensionAction[];
  citationActions: ExtensionAction[];
  documentTasks: ExtensionAction[];
  /** Buttons on the display equations SpeedyF detects on a page. */
  equationActions: ExtensionAction[];
  /** Hosts it may reach through `net.fetch`; empty for one that never
   * touches the network. Shown on its card whether it is on or off. */
  network: string[];
  settings: ExtensionSetting[];
  keybindings: ExtensionKeybinding[];
  /** The bare command id SpeedyF invokes to read a library document's
   * reference list, when this extension is a reference provider. */
  referenceProvider: string | null;
  /** The bare command id SpeedyF invokes for the terms the open document
   * defines, when this extension is a term provider. */
  termProvider: string | null;
  readerPanes: ExtensionPane[];
  projectViews: ExtensionView[];
  /** The bare command invoked for a line in the Projects item detail. */
  itemDetailLines: string | null;
  /** The bare command invoked to read an equation as LaTeX for whoever
   * asks the core (`equations.convert`). */
  equationConverter: string | null;
  iconUrl: string | null;
  /** Why it cannot run: a bad manifest, or a worker that died. */
  error: string | null;
  /** Most recent diagnostics, newest last. Bounded — an extension logging in a
   * loop must not grow the store without limit. */
  log: { level: string; message: string }[];
}

const MAX_LOG_LINES = 50;

interface State {
  loaded: boolean;
  extensions: ExtensionRecord[];
  /** Latest description each panel sent, keyed by qualified panel id. A panel
   * with no entry has not rendered yet, which the sidebar shows as loading
   * rather than as empty. */
  panelUi: Record<string, UiDescription | undefined>;
  /** Which document each rendered panel describes, keyed the same way.
   *
   * Panel content is global while a panel component is not: every tab mounts
   * its own sidebar, so switching documents disposes one `ExtensionPanelView`
   * and mounts a fresh one that finds this store already populated — with the
   * *previous* document's rows. Without a record of what the rows are about,
   * the new panel has no way to tell content from staleness, and it showed the
   * old paper's references indefinitely. */
  panelDoc: Record<string, number | null | undefined>;
}

const [state, setState] = createStore<State>({
  loaded: false,
  extensions: [],
  panelUi: {},
  panelDoc: {},
});

export { state as extensionState };

/** Live workers, keyed by extension id. Not in the store: a Worker is not
 * serializable state and nothing should re-render because one started. */
const hosts = new Map<string, ExtensionHost>();

interface ExtensionDto {
  id: string;
  name: string;
  version: string;
  description?: string;
  source: string;
  permissions: string[];
  enabled: boolean;
  builtin?: boolean;
  manifest:
    | (ExtensionManifest & {
        contributes?: {
          keybindings?: ExtensionKeybinding[];
          readerPanes?: {
            id: string;
            title: string;
            when?: string | null;
            key?: string | null;
            editor: ExtensionEditor;
          }[];
          projectViews?: { id: string; title: string; editor: ExtensionEditor }[];
          itemDetailLines?: { command: string } | null;
          equationConverter?: { command: string } | null;
        };
      })
    | null;
  commands: ExtensionCommand[];
  panels: ExtensionPanel[];
  selectionActions: ExtensionAction[];
  citationActions: ExtensionAction[];
  documentTasks: ExtensionAction[];
  equationActions?: ExtensionAction[];
  network?: string[];
  settings: ExtensionSetting[];
  referenceProvider?: string | null;
  termProvider?: string | null;
  iconUrl: string | null;
  error: string | null;
}

export async function loadExtensions(): Promise<void> {
  try {
    const list = await invoke<ExtensionDto[]>('list_extensions');
    setState(
      produce((s) => {
        // Keep the log of anything still installed; it is the only record of
        // why an extension misbehaved, and a refresh should not erase it.
        const previous = new Map(s.extensions.map((e) => [e.id, e.log]));
        s.extensions = list.map((dto) => ({
          ...dto,
          panels: dto.panels ?? [],
          selectionActions: dto.selectionActions ?? [],
          citationActions: dto.citationActions ?? [],
          documentTasks: dto.documentTasks ?? [],
          equationActions: dto.equationActions ?? [],
          network: dto.network ?? [],
          settings: dto.settings ?? [],
          iconUrl: dto.iconUrl ?? null,
          builtin: dto.builtin ?? false,
          keybindings: dto.manifest?.contributes?.keybindings ?? [],
          referenceProvider: dto.referenceProvider ?? null,
          termProvider: dto.termProvider ?? null,
          ...surfacesOf(dto),
          error: dto.error ?? null,
          log: previous.get(dto.id) ?? [],
        }));
        s.loaded = true;
      })
    );
    syncReferenceProvider();
    syncTermProvider();
  } catch (err) {
    setState('loaded', true);
    console.error('could not list extensions', err);
  }
}

/** The panes, views and slots a manifest declares, namespaced the way
 * commands are. */
function surfacesOf(
  dto: ExtensionDto
): Pick<ExtensionRecord, 'readerPanes' | 'projectViews' | 'itemDetailLines' | 'equationConverter'> {
  const contributes = dto.manifest?.contributes;
  return {
    readerPanes: (contributes?.readerPanes ?? []).map((pane) => ({
      id: `${dto.id}/${pane.id}`,
      localId: pane.id,
      title: pane.title,
      extensionId: dto.id,
      when: pane.when ?? null,
      key: pane.key ?? null,
      editor: { file: pane.editor.file, command: pane.editor.command },
    })),
    projectViews: (contributes?.projectViews ?? []).map((view) => ({
      id: `${dto.id}/${view.id}`,
      localId: view.id,
      title: view.title,
      extensionId: dto.id,
      editor: { file: view.editor.file, command: view.editor.command },
    })),
    itemDetailLines: contributes?.itemDetailLines?.command ?? null,
    equationConverter: contributes?.equationConverter?.command ?? null,
  };
}

/** Reads library documents' reference lists through the active provider. */
const referenceQueue = new ReferenceQueue({
  setProvider: (provider) => invoke('library_refs_provider', { provider }),
  pending: (limit) => invoke('library_refs_pending', { limit }),
  store: (token, entries) => invoke('library_refs_store', { token, entries }),
  invoke: async (choice, value, timeoutMs) => {
    const extension = record(choice.extensionId);
    if (!extension?.enabled) throw new Error('the reference provider is not enabled');
    await hostFor(extension).invoke(choice.command, { value, timeoutMs });
  },
  busy: () => readerBusy(),
  wait: idleWait,
  log: (id, level, message) => appendLog(id, level, message),
});

/** Point the engine at the active reference provider: the first enabled
 * extension declaring one. Run after every change to the extensions, so
 * disabling References turns title matching off and enabling it (or a new
 * version) reads the library again. */
function syncReferenceProvider(): void {
  void referenceQueue.setProvider(chooseProvider(state.extensions));
}

/** Terms the open document defines, through the active term provider. */
const termSession = new TermSession({
  invoke: async (choice, value) => {
    const extension = record(choice.extensionId);
    if (!extension?.enabled) throw new Error('the term provider is not enabled');
    await hostFor(extension).invoke(choice.command, { value });
  },
  store: setDocumentTerms,
  clearAll: clearAllTerms,
  log: (id, level, message) => appendLog(id, level, message),
});

let tracking = false;

/** Point term hovers at the active term provider - the first enabled
 * extension declaring one - and, the first time, start telling it which
 * document is on screen. Disabling it takes every term back. */
function syncTermProvider(): void {
  if (!tracking) {
    tracking = true;
    createRoot(() =>
      createEffect(() => {
        const doc = activeTab()?.documentStore.state;
        termSession.noteActive(doc?.loaded ? doc.docId : null);
      })
    );
  }
  termSession.setProvider(chooseTermProvider(state.extensions));
}

/** Batch jobs extensions offer over a whole document. */
export function documentTasks(): ExtensionAction[] {
  return state.extensions.filter((e) => e.enabled && !e.error).flatMap((e) => e.documentTasks);
}

/** Actions offered on a text selection, from every usable extension, whose
 * `when` holds. */
export function selectionActions(): ExtensionAction[] {
  return state.extensions
    .filter((e) => e.enabled && !e.error)
    .flatMap((e) => e.selectionActions)
    .filter((a) => whenHolds(a.when));
}

/** Actions offered on a detected display equation. Empty — and so no
 * equation buttons at all — unless an enabled extension contributes one. */
export function equationActions(): ExtensionAction[] {
  return state.extensions
    .filter((e) => e.enabled && !e.error)
    .flatMap((e) => e.equationActions)
    .filter((a) => whenHolds(a.when));
}

/** Reader drawers from usable extensions whose `when` holds now. */
export function readerPanes(): ExtensionPane[] {
  return state.extensions
    .filter((e) => e.enabled && !e.error)
    .flatMap((e) => e.readerPanes)
    .filter((p) => whenHolds(p.when));
}

/** Every reader drawer a usable extension declares, `when` or not: the
 * chords are reserved whether or not the drawer is offered right now. */
export function allReaderPanes(): ExtensionPane[] {
  return state.extensions.filter((e) => e.enabled && !e.error).flatMap((e) => e.readerPanes);
}

/** Views of a project from usable extensions. */
export function projectViews(): ExtensionView[] {
  return state.extensions.filter((e) => e.enabled && !e.error).flatMap((e) => e.projectViews);
}

/** Whether an extension is installed, switched on and running clean. */
export function extensionUsable(id: string): boolean {
  const extension = record(id);
  return !!extension && extension.enabled && !extension.error;
}

/** Tell an editor's extension what happened in it. Failures are logged
 * against the extension, never thrown at the editor. */
export async function runEditorEvent(
  extensionId: string,
  editor: ExtensionEditor,
  value: EditorEvent
): Promise<void> {
  const extension = record(extensionId);
  if (!extension || !extension.enabled || extension.error) return;
  try {
    await hostFor(extension).invoke(editor.command, { value });
  } catch (err) {
    appendLog(extensionId, 'error', `editor: ${err instanceof Error ? err.message : String(err)}`);
  }
}

/** Ask every extension with an item line about the item the Projects
 * detail column shows. They answer with `ui.itemLine`. */
export async function requestItemLines(request: ItemLineRequest): Promise<void> {
  const askers = state.extensions.filter(
    (e) => e.enabled && !e.error && e.itemDetailLines !== null
  );
  await Promise.all(
    askers.map(async (extension) => {
      try {
        await hostFor(extension).invoke(extension.itemDetailLines!, { value: request });
      } catch (err) {
        appendLog(
          extension.id,
          'error',
          `item line: ${err instanceof Error ? err.message : String(err)}`
        );
      }
    })
  );
}

/** Run the action of a line an extension put in the item detail. */
export async function runItemLineAction(line: ItemLine): Promise<void> {
  if (!line.command) return;
  await runPanelAction(line.extensionId, line.command, line.value as InvokeValue | undefined);
}

/** Extensions waiting on a converter's `ui.showMath`, by converter id. */
const converting = new Map<string, (answer: { latex: string; display: boolean }) => void>();

/** An equation's LaTeX from the first enabled extension declaring an
 * `equationConverter`: invoked as an equation action would be, its
 * `ui.showMath` taken as the answer instead of shown. Null when there is no
 * converter, it fails, or it answers nothing. */
export async function convertEquation(
  token: string,
  ref: EquationRef
): Promise<{ latex: string; display: boolean; converter: string } | null> {
  const extension = state.extensions.find(
    (e) => e.enabled && !e.error && e.equationConverter !== null
  );
  if (!extension) return null;
  let answer: { latex: string; display: boolean } | null = null;
  converting.set(extension.id, (value) => {
    answer = value;
  });
  try {
    await hostFor(extension).invoke(extension.equationConverter!, {
      value: { page: ref.page, bbox: ref.bbox, number: ref.number, equation: token },
      timeoutMs: invokeTimeout(extension),
    });
  } catch (err) {
    appendLog(
      extension.id,
      'info',
      `converter: ${err instanceof Error ? err.message : String(err)}`
    );
  } finally {
    converting.delete(extension.id);
  }
  const got = answer as { latex: string; display: boolean } | null;
  return got ? { ...got, converter: extension.name } : null;
}

/** Actions offered on a citation. */
export function citationActions(): ExtensionAction[] {
  return state.extensions.filter((e) => e.enabled && !e.error).flatMap((e) => e.citationActions);
}

/** Run an action on something the user picked.
 *
 * The picked thing is passed as the invoke value and is the *only* thing the
 * extension receives. That is what makes actions the narrow path: an extension
 * acting on a selection needs no read permission at all, because the user chose
 * both the text and the action.
 */
export async function runExtensionAction(action: ExtensionAction, value: string): Promise<void> {
  const extension = state.extensions.find((e) => e.id === action.extensionId);
  if (!extension || !extension.enabled) return;
  const onSelection = extension.selectionActions.some((a) => a.id === action.id);
  await invokeAction(
    extension,
    action.localId,
    action.title,
    value,
    onSelection ? selectionAnchor() : null
  );
}

/** Run an equation action on the block the user pointed at. `anchor` is the
 * block on screen, where the answer appears. */
export async function runEquationAction(
  action: ExtensionAction,
  target: EquationTarget,
  anchor: MathAnchor
): Promise<void> {
  const extension = state.extensions.find((e) => e.id === action.extensionId);
  if (!extension || !extension.enabled) return;
  const docId = activeTab()?.documentStore.state.docId ?? null;
  const equation =
    docId !== null
      ? equationToken({ docId, page: target.page, bbox: target.bbox, number: target.number })
      : undefined;
  const value = equation ? { ...target, equation } : target;
  // An extension that answers with a formula gets the popover; one that
  // does something else with the equation (Notepad) does not.
  const popover = extension.permissions.includes('document:layout');
  await invokeAction(
    extension,
    action.localId,
    action.title,
    value,
    popover ? anchor : null,
    popover
  );
}

/** Invoke an action, with the math popover around it for an extension that
 * reads layout: it opens loading at `anchor`, `ui.showMath` fills it, and a
 * failure shows in it rather than only in the extension's log. An equation
 * action always has the popover — it is the only thing it can answer with. */
async function invokeAction(
  extension: ExtensionRecord,
  command: string,
  title: string,
  value: InvokeValue,
  anchor: MathAnchor | null,
  always = false
): Promise<void> {
  const popover = anchor !== null && (always || extension.permissions.includes('document:layout'));
  if (popover) openMathLoading(extension.id, anchor, title);
  try {
    await hostFor(extension).invoke(command, { value, timeoutMs: invokeTimeout(extension) });
    if (popover) settleMath(extension.id);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    appendLog(extension.id, 'error', `${title}: ${message}`);
    if (popover) showMathError(extension.id, message);
  }
}

/** An extension that waits on the network gets longer to answer: its work is
 * paced per host, and only its own popover waits on it. */
function invokeTimeout(extension: ExtensionRecord): number {
  return extension.permissions.includes('network') ? NETWORK_INVOKE_TIMEOUT_MS : INVOKE_TIMEOUT_MS;
}

/** The current selection on screen, for a popover about it: the union of
 * its rectangles on its page, mapped through the page element. On a rotated
 * page the page's top edge stands in. Null when nothing is selected in the
 * active document or its page is not mounted. */
function selectionAnchor(): MathAnchor | null {
  const geometry = selectedGeometry();
  const tab = activeTab();
  if (!geometry || !tab || geometry.docId !== tab.documentStore.state.docId) return null;
  const geom = tab.zoom.pagesGeom()[geometry.page];
  if (!geom || geometry.rects.length === 0) return null;
  const element = [...document.querySelectorAll(`.page[data-page-index="${geometry.page}"]`)].find(
    (el) => el.getClientRects().length > 0
  );
  if (!element) return null;
  const box = element.getBoundingClientRect();
  if (geom.rotation !== 0)
    return { left: box.left, top: box.top, right: box.right, bottom: box.top };
  const scale = box.width / geom.widthPt;
  const left = Math.min(...geometry.rects.map((r) => r[0]));
  const top = Math.min(...geometry.rects.map((r) => r[1]));
  const right = Math.max(...geometry.rects.map((r) => r[0] + r[2]));
  const bottom = Math.max(...geometry.rects.map((r) => r[1] + r[3]));
  return {
    left: box.left + left * scale,
    top: box.top + top * scale,
    right: box.left + right * scale,
    bottom: box.top + bottom * scale,
  };
}

/** Requests in flight per extension id, for the status bar. */
const [networkActivity, setNetworkActivity] = createStore<Record<string, number>>({});

/** Names of the extensions using the internet right now. */
export function extensionsOnline(): string[] {
  return Object.entries(networkActivity)
    .filter(([, count]) => count > 0)
    .map(([id]) => record(id)?.name ?? id);
}

/** `net.fetch`: the core makes the request; the worker never connects. */
async function netFetch(extension: ExtensionRecord, args: unknown): Promise<unknown> {
  const { url, kind, maxBytes } = readFetchArgs(args);
  setNetworkActivity(extension.id, (count) => (count ?? 0) + 1);
  try {
    return await invoke('extension_net_fetch', {
      extensionId: extension.id,
      url,
      kind,
      maxBytes,
    });
  } finally {
    setNetworkActivity(extension.id, (count) => Math.max(0, (count ?? 1) - 1));
  }
}

/** Every panel contributed by an enabled, error-free extension. */
export function extensionPanels(): ExtensionPanel[] {
  return state.extensions.filter((e) => e.enabled && !e.error).flatMap((e) => e.panels);
}

export function panelUi(panelId: string): UiDescription | undefined {
  return state.panelUi[panelId];
}

/** The document a panel's current content was rendered for.
 *
 * `undefined` means nothing has rendered yet. Compare against the open
 * document to know whether what is on screen is about the paper being read. */
export function panelDocId(panelId: string): number | null | undefined {
  return state.panelDoc[panelId];
}

/** The open document, as panel content is stamped with. */
function currentDocId(): number | null {
  return activeTab()?.documentStore.state.docId ?? null;
}

/** Record a panel's content together with what it describes. Every write to
 * `panelUi` goes through here, so the two can never drift apart. */
function setPanelUi(panelId: string, description: UiDescription): void {
  setState('panelUi', panelId, description);
  setState('panelDoc', panelId, currentDocId());
}

/** Ask a panel's extension to render it.
 *
 * Called when the panel is opened and when the user asks for a refresh, rather
 * than on a timer: an extension should not be woken because a document scrolled.
 */
export async function refreshPanel(panelId: string): Promise<void> {
  const panel = extensionPanels().find((p) => p.id === panelId);
  if (!panel) return;
  const extension = state.extensions.find((e) => e.id === panel.extensionId);
  if (!extension) return;
  try {
    await hostFor(extension).invoke(panel.localId, { reason: 'panel' });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    appendLog(extension.id, 'error', `${panel.title}: ${message}`);
    setPanelUi(panelId, { sections: [], error: message });
  }
}

/** Every command contributed by an enabled, error-free extension, whose
 * `when` holds. */
export function extensionCommands(): ExtensionCommand[] {
  return state.extensions
    .filter((e) => e.enabled && !e.error)
    .flatMap((e) => e.commands)
    .filter((c) => whenHolds(c.when));
}

/** Contributed keybindings that survived conflict resolution.
 *
 * Recomputed from the store rather than cached, so enabling or removing an
 * extension takes effect on the next keypress with nothing to invalidate.
 */
export function extensionBindings(): ExtensionBinding[] {
  const declarations = state.extensions
    .filter((e) => e.enabled && !e.error && e.keybindings.length > 0)
    .map((e) => ({ extensionId: e.id, bindings: e.keybindings }));
  if (declarations.length === 0) return [];
  return resolveBindings(declarations, reservedChords()).bindings;
}

/** Bindings an extension declared that were refused, for the panel to explain. */
export function bindingErrors(id: string): string[] {
  const extension = state.extensions.find((e) => e.id === id);
  if (!extension || extension.keybindings.length === 0) return [];
  const declarations = state.extensions
    .filter((e) => e.enabled && !e.error && e.keybindings.length > 0)
    .map((e) => ({ extensionId: e.id, bindings: e.keybindings }));
  return resolveBindings(declarations, reservedChords())
    .errors.filter((e) => e.extensionId === id)
    .map((e) => e.message);
}

export function findExtensionForCommand(qualifiedId: string): ExtensionRecord | undefined {
  return state.extensions.find((e) => e.commands.some((c) => c.id === qualifiedId));
}

function record(id: string): ExtensionRecord | undefined {
  return state.extensions.find((e) => e.id === id);
}

function appendLog(id: string, level: string, message: string): void {
  setState(
    'extensions',
    (e) => e.id === id,
    produce((e: ExtensionRecord) => {
      e.log.push({ level, message });
      if (e.log.length > MAX_LOG_LINES) e.log.splice(0, e.log.length - MAX_LOG_LINES);
    })
  );
}

function setError(id: string, error: string | null): void {
  setState('extensions', (e) => e.id === id, 'error', error);
}

/** The permission gate: the whole security boundary between an extension and
 * the open document.
 *
 * Refusing is the default and always has been — an API only becomes reachable
 * by being named here. That ordering matters more than it looks: opening a
 * capability later breaks nothing, while discovering that extensions already
 * had one cannot be undone for anything already published against it.
 */
async function handleCall(extension: ExtensionRecord, call: CallMessage): Promise<unknown> {
  if (call.api === 'references.contribute') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    return referenceQueue.contribute(extension.id, call.args);
  }

  if (call.api === 'terms.contribute') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    const doc = activeTab()?.documentStore.state;
    const view = doc?.loaded
      ? {
          docId: doc.docId,
          pageCount: doc.pages.length,
          srcOf: (page: number) => doc.pages[page]?.srcIndex ?? null,
        }
      : null;
    return termSession.contribute(extension.id, call.args, view);
  }

  if (call.api === 'document.contributeText') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    const tab = activeTab();
    const doc = tab?.documentStore.state;
    if (!tab || !doc?.loaded) throw new Error('no document is open');
    const args = call.args as { page?: unknown; text?: unknown } | undefined;
    const page = readPageArg(args, doc.pages.length);
    if (typeof args?.text !== 'string') throw new Error('expected a string "text"');
    if (args.text.length > MAX_CONTRIBUTED_TEXT) {
      throw new Error(`contributed text is limited to ${MAX_CONTRIBUTED_TEXT} characters a page`);
    }
    await invoke('extension_contribute_text', {
      extensionId: extension.id,
      document: doc.path,
      page,
      text: args.text,
    });
    return { stored: args.text.length };
  }

  if (call.api === 'settings.get') {
    // No permission: these are the extension's own declared settings, and
    // reading what the user chose to tell it is not access to anything else.
    const key = (call.args as { key?: unknown } | undefined)?.key;
    if (typeof key !== 'string') throw new Error('settings.get needs a string "key"');
    if (!extension.settings.some((s) => s.key === key)) {
      throw new Error(`this extension does not declare a setting "${key}"`);
    }
    return settingValue(extension, key);
  }

  if (call.api === 'clipboard.write') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    const text = (call.args as { text?: unknown } | undefined)?.text;
    if (typeof text !== 'string' || text.length === 0) {
      throw new Error('clipboard.write needs a non-empty string "text"');
    }
    if (text.length > MAX_CLIPBOARD) {
      throw new Error(`clipboard.write is limited to ${MAX_CLIPBOARD} characters`);
    }
    try {
      await navigator.clipboard.writeText(text);
    } catch (err) {
      // The platform requires transient user activation for a clipboard write.
      // Every real path here has it — an extension command is reached by a
      // click or a keypress — but the round trip out to the worker and back
      // spends some of that window, and a slow extension can outlast it. Named
      // explicitly because the raw platform message ("not allowed by the user
      // agent") sends an author looking for a permission that does not exist.
      throw new Error(
        'the clipboard is only writable while a user action is still in effect; ' +
          `do the write first, before any slow work (${err instanceof Error ? err.message : String(err)})`
      );
    }
    return { written: text.length };
  }

  if (call.api.startsWith('storage.')) {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    return storageCall(extension.id, call.api, call.args);
  }

  if (call.api === 'ui.showMath') {
    // No permission: the popover is SpeedyF's, and only a formula goes in it.
    const args = readShowMathArgs(call.args);
    const capture = converting.get(extension.id);
    if (capture) {
      // Invoked as an equation converter: the formula is the answer.
      capture({ latex: args.latex, display: args.display });
      return { shown: true };
    }
    showMath(extension.id, args, selectionAnchor());
    return { shown: true };
  }

  if (call.api in API_PERMISSIONS && API_PERMISSIONS[call.api]?.startsWith('projects:')) {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    return projectsApi(extension, call.api, call.args, () => hostFor(extension));
  }

  if (call.api === 'region.capture' || call.api === 'equations.snapshot') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    if (call.api === 'equations.snapshot') {
      return snapshotRegion(equationFor((call.args as { equation?: unknown })?.equation));
    }
    const resume = hostFor(extension).pauseDeadlines();
    try {
      return await captureRegion(
        (call.args as { prompt?: unknown } | undefined)?.prompt,
        extension.name
      );
    } finally {
      resume();
    }
  }

  if (OPEN_APIS.has(call.api)) {
    // A conversion waits on another extension, which may be fetching the
    // paper's source: not this extension's time.
    const resume = call.api === 'equations.convert' ? hostFor(extension).pauseDeadlines() : null;
    try {
      return await openApi(extension, call.api, call.args, convertEquation);
    } finally {
      resume?.();
    }
  }

  if (call.api === 'net.fetch') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    return netFetch(extension, call.args);
  }

  if (call.api === 'ui.render') {
    const args = call.args as { panel?: unknown; ui?: unknown } | undefined;
    const localId = typeof args?.panel === 'string' ? args.panel : null;
    if (!localId) throw new Error('ui.render needs a string "panel"');
    const panel = extension.panels.find((p) => p.localId === localId);
    if (!panel) throw new Error(`this extension does not contribute a panel "${localId}"`);
    // Validated, never cast: this is the one place an extension puts text in
    // front of the user, and the renderer must not receive a half-typed object.
    setPanelUi(panel.id, parseUi(args?.ui));
    return { rendered: true };
  }

  if (call.api !== 'commands.run') {
    if (!canCallApi(extension.permissions, call.api)) {
      throw new Error(apiRefusalReason(extension.permissions, call.api));
    }
    return readApi(call.api, call.args);
  }

  if (call.api === 'commands.run') {
    const id = (call.args as { id?: unknown } | undefined)?.id;
    if (typeof id !== 'string') throw new Error('commands.run needs a string "id"');
    const group = COMMANDS.find((c) => c.id === id)?.group;
    if (!canRunGroup(extension.permissions, group)) {
      throw new Error(refusalReason(extension.permissions, id, group));
    }
    // `runCommand` returns false when the command exists but is not currently
    // available — no document open, nothing selected. Reported rather than
    // thrown: it is a state the extension can reasonably retry from.
    return { ran: runCommand(id, { tab: activeTab() }) };
  }

  throw new Error(
    `"${call.api}" is not available to extensions in this version of SpeedyF` +
      (extension.permissions.length === 0 ? '' : ` (granted: ${extension.permissions.join(', ')})`)
  );
}

/** Longest string an extension may put on the clipboard. Generous for a
 * bibliography, far short of a whole document. */
const MAX_CLIPBOARD = 100_000;
/** Per-key and per-extension storage limits. localStorage is a few megabytes
 * shared with the app's own settings, so an extension gets a small, bounded
 * slice of it rather than whatever it can take. */
const MAX_STORAGE_VALUE = 64_000;
/** A page of dense text is a few thousand characters; this leaves room for
 * layout markup an OCR engine might include without letting a runaway loop
 * write a document-sized string per page. */
const MAX_CONTRIBUTED_TEXT = 200_000;
const MAX_STORAGE_KEYS = 100;

function storageKey(extensionId: string, key: string): string {
  return `speedyf-ext:${extensionId}:${key}`;
}

/** Ticks whenever any setting changes.
 *
 * Setting values live in localStorage, which is not reactive, and the store
 * holds the *declarations* rather than the values. Anything displaying a value
 * reads this first so it re-runs when one is written. */
const [settingsRevision, bumpSettingsRevision] = createSignal(0);

export { settingsRevision };

/** Settings share the storage namespace but a separate prefix, so an
 * extension's own `storage.set` can never overwrite a value the user chose in
 * the settings form. */
function settingKey(extensionId: string, key: string): string {
  return `speedyf-ext-setting:${extensionId}:${key}`;
}

/** What a setting is currently worth: what the user chose, or the declared
 * default. */
export function settingValue(extension: ExtensionRecord, key: string): string {
  settingsRevision(); // re-read when any setting is written
  const stored = localStorage.getItem(settingKey(extension.id, key));
  if (stored !== null) return stored;
  return extension.settings.find((s) => s.key === key)?.default ?? '';
}

export function setSettingValue(extensionId: string, key: string, value: string): void {
  try {
    localStorage.setItem(settingKey(extensionId, key), value);
  } catch {
    /* storage unavailable — the setting stays session-only */
  }
  // The worker holds no copy, so it sees the new value on its next read. A
  // restart would be heavier and would lose whatever state it had built up.
  bumpSettingsRevision((n) => n + 1);
}

function clearSettings(extensionId: string): void {
  const prefix = settingKey(extensionId, '');
  const doomed: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (key?.startsWith(prefix)) doomed.push(key);
  }
  for (const key of doomed) localStorage.removeItem(key);
}

/** Namespaced key/value storage.
 *
 * The namespace is applied here, not by the extension, so one extension cannot
 * read or overwrite another's — and uninstalling can find everything an
 * extension left behind by prefix.
 */
function storageCall(extensionId: string, api: string, args: unknown): unknown {
  const key = (args as { key?: unknown } | undefined)?.key;
  if (typeof key !== 'string' || key.length === 0 || key.length > 200) {
    throw new Error('storage needs a "key" of 1-200 characters');
  }
  const full = storageKey(extensionId, key);

  if (api === 'storage.get') return localStorage.getItem(full);

  if (api === 'storage.remove') {
    localStorage.removeItem(full);
    return { removed: true };
  }

  if (api === 'storage.set') {
    const value = (args as { value?: unknown } | undefined)?.value;
    if (typeof value !== 'string') throw new Error('storage.set needs a string "value"');
    if (value.length > MAX_STORAGE_VALUE) {
      throw new Error(`storage values are limited to ${MAX_STORAGE_VALUE} characters`);
    }
    const prefix = storageKey(extensionId, '');
    if (localStorage.getItem(full) === null) {
      let used = 0;
      for (let i = 0; i < localStorage.length; i++) {
        if (localStorage.key(i)?.startsWith(prefix)) used++;
      }
      if (used >= MAX_STORAGE_KEYS) {
        throw new Error(`an extension may keep at most ${MAX_STORAGE_KEYS} stored keys`);
      }
    }
    localStorage.setItem(full, value);
    return { stored: value.length };
  }

  throw new Error(`"${api}" is not a storage operation`);
}

/** Remove everything an extension stored. Called on uninstall: leaving an
 * uninstalled extension's data behind would be a surprise, and it is the one
 * cleanup nothing else can do later once the id is gone. */
function clearStorage(extensionId: string): void {
  const prefix = storageKey(extensionId, '');
  const doomed: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (key?.startsWith(prefix)) doomed.push(key);
  }
  for (const key of doomed) localStorage.removeItem(key);
}

/** Dispatch a read API, after the gate has allowed it.
 *
 * Everything is scoped to the *active* document. An extension never names a
 * document, which keeps engine ids out of the surface entirely and means an
 * extension cannot read a tab the user is not looking at.
 */
async function readApi(api: string, args: unknown): Promise<unknown> {
  if (api === 'render.page') {
    const tab = activeTab();
    const doc = tab?.documentStore.state;
    if (!tab || !doc?.loaded) throw new Error('no document is open');
    const page = readPageArg(args, doc.pages.length);
    const src = doc.pages[page]?.srcIndex;
    if (src === undefined || src === null) throw new Error(`page ${page} has no source page`);
    const scale = (args as { scaleMilli?: unknown }).scaleMilli;
    const bytes = await invoke<ArrayBuffer>('extension_render_page', {
      docId: doc.docId,
      src,
      scaleMilli: typeof scale === 'number' ? Math.round(scale) : 2000,
    });
    // Handed over as plain numbers. An ArrayBuffer would be cheaper, but it is
    // a *transferable*: posting one detaches it here, and the host would be
    // handing away memory it does not own on behalf of a caller that cannot be
    // trusted to have finished with it.
    return Array.from(new Uint8Array(bytes));
  }

  // The library APIs work with nothing open — the library is not a property
  // of the current document.
  if (api === 'library.search') {
    return toLibraryHits(await engine.librarySearch(readQueryArg(args), false));
  }
  if (api === 'library.papers') return toPapers(await engine.libraryPapers({}));
  if (api === 'library.projects') return toProjects(await engine.libraryProjects());
  if (api === 'library.paper') return toPaper(await engine.libraryPaper(readIdArg(args)));

  const tab = activeTab();
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded) throw new Error('no document is open');

  switch (api) {
    case 'navigate.toPage': {
      const page = readPageArg(args, doc.pages.length);
      jumpToPage(tab, page);
      return { page };
    }

    case 'navigate.toPlace': {
      const { page, boxes, y } = readPlaceArg(args, doc.pages.length);
      // Marked before scrolling, so the mark is already there when the page
      // arrives rather than appearing a moment after it settles.
      revealPlace(
        doc.docId,
        page,
        boxes.map((box) => [box.x, box.y - box.h, box.w, box.h] as [number, number, number, number])
      );
      const geom = tab.zoom.pagesGeom()[page];
      if (!geom) {
        // Geometry is only known for pages the viewer has measured. Landing on
        // the page is still the right thing to do; landing nowhere is not.
        jumpToPage(tab, page);
        return { page };
      }
      const css = pdfRectToCssRect({ x: 0, y, w: 0, h: 0 }, geom, tab.viewport.state.zoom);
      // Framed a third of the way down rather than at the very top, so the
      // line that cites the work has its context above it. Never above the
      // page's own top: for a citation in the first few lines the framing
      // offset goes negative, which lands on the end of the previous page.
      jumpToPage(tab, page, Math.max(0, css.y - tab.viewport.state.containerH * 0.35));
      return { page, y };
    }

    case 'document.info':
      return toDocumentInfo(doc.name, doc.pages.length, tab.viewport.state.currentPage);

    case 'document.outline':
      return toOutline(await engine.getOutline(doc.docId));

    case 'document.figures':
      return toFigures(await engine.getFigures(doc.docId));

    case 'document.pageText': {
      const page = readPageArg(args, doc.pages.length);
      // Extensions address pages as the reader sees them; `srcIndex` maps that
      // to the page in the file, which reordering and deletion move around.
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return '';
      return toPageText(await engine.getTextLayout(doc.docId, src));
    }

    case 'document.links': {
      const page = readPageArg(args, doc.pages.length);
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return [];
      return (await engine.getPageLinks(doc.docId, src).then(toLinks)).map((link) => ({
        ...link,
        page,
      }));
    }

    case 'document.jumps': {
      const page = readPageArg(args, doc.pages.length);
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return [];
      // Destinations come back as pages in the *file*. An extension addresses
      // pages as the reader sees them, so a jump into a page that reordering
      // moved must be reported at its new index, and one into a page that was
      // deleted must not be reported at all — a row that navigates nowhere is
      // worse than a row that is missing.
      const toReader = new Map<number, number>();
      doc.pages.forEach((p, index) => {
        if (p.srcIndex !== undefined && p.srcIndex !== null) toReader.set(p.srcIndex, index);
      });
      return toJumps(await engine.getPageLinks(doc.docId, src))
        .map((jump) => ({ ...jump, page, toPage: toReader.get(jump.toPage) ?? -1 }))
        .filter((jump) => jump.toPage >= 0);
    }

    case 'document.pageRuns': {
      const page = readPageArg(args, doc.pages.length);
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return [];
      return toPageRuns(await engine.getTextLayout(doc.docId, src));
    }

    case 'citations.list': {
      const page = readPageArg(args, doc.pages.length);
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return [];
      return (await engine.getPageLinks(doc.docId, src).then(toCitations)).map((citation) => ({
        ...citation,
        page,
      }));
    }

    case 'citations.resolve':
      // The engine answers with the cite key of the paper holding the file,
      // so a panel resolving a whole bibliography makes one call per work,
      // not two. A file the paper layer cannot place simply has none.
      return toResolvedCitation(await engine.resolveCitation(readCitationArg(args)));

    case 'document.contributedText':
      return invoke('extension_contributed_text', { document: doc.path });

    case 'selection.glyphs': {
      const geometry = selectedGeometry();
      if (!geometry || geometry.docId !== doc.docId) return null;
      const src = doc.pages[geometry.page]?.srcIndex;
      if (src === undefined || src === null) return null;
      const layout = await invoke<Parameters<typeof toGlyphs>[1]>('get_page_glyphs', {
        docId: doc.docId,
        src,
        rects: geometry.rects,
      });
      return toGlyphs(geometry.page, layout);
    }

    case 'region.glyphs': {
      const { page, bbox } = readRegionArg(args, doc.pages.length);
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return { page, glyphs: [] };
      // An equation's block: the engine grows it to everything attached
      // (numerators over bars, the left-hand side on its baseline).
      const layout = await invoke<Parameters<typeof toGlyphs>[1]>('get_page_glyphs', {
        docId: doc.docId,
        src,
        rects: [bbox],
        grow: true,
      });
      return toGlyphs(page, layout);
    }

    case 'document.mathTags': {
      const page = readPageArg(args, doc.pages.length);
      const src = doc.pages[page]?.srcIndex;
      if (src === undefined || src === null) return [];
      return toMathTags(
        await invoke<Parameters<typeof toMathTags>[0]>('get_math_tags', { docId: doc.docId, src })
      );
    }

    case 'document.identifiers':
      return invoke('extension_document_identifiers', {
        docId: doc.docId,
        path: doc.path ?? null,
      });
  }

  // Unreachable: `canCallApi` already refused anything not in the catalogue.
  throw new Error(`"${api}" is not an API this version of SpeedyF offers`);
}

function hostFor(extension: ExtensionRecord): ExtensionHost {
  const existing = hosts.get(extension.id);
  if (existing) return existing;

  const host = new ExtensionHost({
    id: extension.id,
    apiVersion: 1,
    permissions: extension.permissions,
    spawn: async () => {
      const source = await invoke<string>('extension_source', { id: extension.id });
      return blobWorkerFactory(source)();
    },
    events: {
      onLog: (level, message) => appendLog(extension.id, level, message),
      onCrash: (reason) => {
        appendLog(extension.id, 'error', reason);
        setError(extension.id, reason);
        hosts.delete(extension.id);
      },
      onCall: (call) => handleCall(extension, call),
    },
  });
  hosts.set(extension.id, host);
  return host;
}

/** Run a contributed command by its namespaced id.
 *
 * Resolves when the extension reports done, and never hangs — the host times
 * out. Failures are recorded against the extension rather than thrown at the
 * caller, because the caller is a palette row that has already closed.
 */
export async function runExtensionCommand(qualifiedId: string, value?: string): Promise<void> {
  const extension = findExtensionForCommand(qualifiedId);
  if (!extension || !extension.enabled) return;
  const command = extension.commands.find((c) => c.id === qualifiedId);
  if (!command) return;

  try {
    await hostFor(extension).invoke(command.localId, value === undefined ? undefined : { value });
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    appendLog(extension.id, 'error', `${command.title}: ${message}`);
  }
}

/** Run a row action from a rendered panel.
 *
 * The command is the extension's own bare id, taken from the description it
 * sent, so it can only ever reach back into itself.
 */
export async function runPanelAction(
  extensionId: string,
  command: string,
  value?: InvokeValue
): Promise<void> {
  const extension = state.extensions.find((e) => e.id === extensionId);
  if (!extension || !extension.enabled) return;
  try {
    await hostFor(extension).invoke(command, value === undefined ? undefined : { value });
  } catch (err) {
    appendLog(extensionId, 'error', err instanceof Error ? err.message : String(err));
  }
}

/** Text extensions have contributed for a document, if any. */
export async function contributedTextFor(document: string): Promise<{
  extensionId: string;
  pages: { page: number; text: string }[];
} | null> {
  return invoke('extension_contributed_text', { document });
}

/** Discard everything extensions contributed for a document.
 *
 * The user's call, never an automatic one. A sidecar outlives the extension
 * that wrote it on purpose — losing a scanned document's text because an
 * extension was uninstalled months later would be a surprising thing for an
 * uninstall to do.
 */
export async function forgetContributedText(document: string): Promise<void> {
  await invoke('forget_contributed_text', { document });
}

/** Run a command reached by its keybinding.
 *
 * A key press is as explicit an instruction as picking a menu item, so a
 * command that is *also* declared as a selection action is given the current
 * selection here, exactly as it would be from the palette. Without this a
 * hotkey for "do something with what I highlighted" fires with nothing
 * attached, which is the obvious thing to bind a key to and the one thing that
 * would not work.
 *
 * The narrow-path property is preserved: the extension still receives only
 * what the user picked, and still needs no read permission for it.
 */
export async function runBoundCommand(extensionId: string, command: string): Promise<void> {
  const extension = state.extensions.find((e) => e.id === extensionId);
  if (!extension || !extension.enabled) return;
  const qualified = `${extensionId}/${command}`;
  const isSelectionAction = extension.selectionActions.some((a) => a.id === qualified);
  const selection = isSelectionAction ? selectedText() : '';
  if (isSelectionAction && selection.length === 0) {
    // Nothing highlighted. Reported against the extension rather than thrown
    // at a keypress nobody is waiting on.
    appendLog(extensionId, 'info', `${command}: nothing is selected`);
    return;
  }
  try {
    if (isSelectionAction) {
      await invokeAction(extension, command, command, selection, selectionAnchor());
      return;
    }
    await hostFor(extension).invoke(command, selection ? { value: selection } : undefined);
  } catch (err) {
    appendLog(extensionId, 'error', err instanceof Error ? err.message : String(err));
  }
}

/** Stop an extension's worker without touching what is installed. */
export function stopExtension(id: string): void {
  hosts.get(id)?.stop();
  hosts.delete(id);
}

export async function setExtensionEnabled(id: string, enabled: boolean): Promise<void> {
  // A disabled extension must not keep running, so the worker goes first —
  // if the write fails, the worse outcome is a stopped extension, not a
  // running one the user believes is off.
  if (!enabled) stopExtension(id);
  await invoke('set_extension_enabled', { id, enabled });
  await loadExtensions();
}

export async function installExtension(folder: string): Promise<void> {
  const installed = await invoke<{ id: string }>('install_extension', { folder });
  await adoptInstalled(installed.id);
}

/** Install from a git URL, optionally pinned with `#<branch|tag|commit>`. */
export async function installExtensionFromGit(url: string): Promise<void> {
  const installed = await invoke<{ id: string }>('install_extension_from_git', { url });
  await adoptInstalled(installed.id);
}

/** Take up a freshly written extension.
 *
 * Stopping the worker is the whole point. Installing over an extension that is
 * already running replaces its files but not the code already loaded into its
 * worker, so without this a reinstall — the entire developer loop — appears to
 * do nothing at all. The worker restarts lazily on the next invoke, with the
 * new source.
 */
async function adoptInstalled(id: string): Promise<void> {
  stopExtension(id);
  setState('panelUi', reconcilePanelUi(id));
  setState('panelDoc', reconcilePanelDoc(id));
  await loadExtensions();
}

/** Drop rendered panel content belonging to `id`, so a reinstalled extension
 * redraws rather than showing what the previous version left behind. */
function reconcilePanelUi(id: string): Record<string, UiDescription | undefined> {
  const next: Record<string, UiDescription | undefined> = {};
  for (const [panelId, ui] of Object.entries(state.panelUi)) {
    if (!panelId.startsWith(`${id}/`)) next[panelId] = ui;
  }
  return next;
}

/** The same, for what each panel's content describes — dropped together with
 * the content so a panel is never left stamped with a document it no longer
 * shows anything about. */
function reconcilePanelDoc(id: string): Record<string, number | null | undefined> {
  const next: Record<string, number | null | undefined> = {};
  for (const [panelId, docId] of Object.entries(state.panelDoc)) {
    if (!panelId.startsWith(`${id}/`)) next[panelId] = docId;
  }
  return next;
}

export async function uninstallExtension(id: string): Promise<void> {
  stopExtension(id);
  clearStorage(id);
  clearSettings(id);
  await invoke('uninstall_extension', { id });
  await loadExtensions();
}

/** Restart a running extension — the developer-loop action, and the way to
 * clear a crash without reinstalling. */
export async function reloadExtension(id: string): Promise<void> {
  stopExtension(id);
  setError(id, null);
  await loadExtensions();
  const extension = record(id);
  if (extension?.enabled) {
    try {
      await hostFor(extension).start();
    } catch {
      /* onCrash has already recorded why */
    }
  }
}
