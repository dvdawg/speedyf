/** The open document as a paper in your library.
 *
 * The sidebar's other panels are about what is *in* the document; this one is
 * about where it *sits*: its record, its cite key, which projects hold it, and
 * — once the graph exists — which of your other papers it cites and which
 * cite it. For a file outside the library folders there is no record to show,
 * so the panel offers to make one by copying the file into the inbox folder
 * (projects.md §6). Never a move: the user's file stays where it was.
 *
 * The record is re-read on every `library:changed`, which is how a membership
 * toggled here, a lookup finished elsewhere, or a change made in another
 * window all reach the panel without it assuming it was the writer. */
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  untrack,
  useContext,
} from 'solid-js';
import { engine, isEngineError } from '../../lib/transport/engine';
import { TabContext } from '../../app/TabContext';
import { projectsStore } from './projectsStore';
import { forgetPapers, rememberPaper } from './activePaper';
import { chooseInbox } from './fileDrop';
import { settings, updateSettings } from '../../stores/settings';
import { goHome, openInNewTabOrFocus } from '../document/tabsController';
import type { Author, CitationLinks, Paper, Paperness, PaperRow } from '../../types/engine';
import MathText from '../../components/MathText';
import ProjectSuggestionLine from './ProjectSuggestion';
import RelatedItems from './RelatedItems';

/** What the engine answers for a call the Rust side has not grown yet. The
 * citation graph is the one section that hides on it rather than reporting
 * it: an empty "In your library" would read as "this paper cites nothing of
 * yours", which is a claim, and a wrong one. */
const NOT_IMPLEMENTED = 'not implemented yet';

/** How authors read in one line: "A", "A & B", "A, B & C", then "A et al." —
 * four names already cost more than they tell. Family names only; a paper
 * with none falls back to the given name rather than an empty slot. */
export function authorsLine(authors: readonly Author[]): string {
  const names = authors
    .map((author) => author.family.trim() || author.given.trim())
    .filter((name) => name.length > 0);
  if (names.length === 0) return '';
  if (names.length === 1) return names[0]!;
  if (names.length === 2) return `${names[0]} & ${names[1]}`;
  if (names.length === 3) return `${names[0]}, ${names[1]} & ${names[2]}`;
  return `${names[0]} et al.`;
}

const message = (error: unknown) =>
  isEngineError(error) ? error.message : error instanceof Error ? error.message : String(error);

const CLIPBOARD_REFUSED =
  'The clipboard refused the write: it is only open for a moment after a click. Press the button again.';

export default function PaperPanel() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;

  /** The record for the open file. `undefined` until the first answer, `null`
   * when the engine says the file is outside the library. */
  const [paper, setPaper] = createSignal<Paper | null | undefined>(undefined);
  /** The copy that Add to library made. Held apart from `paper` because the
   * open document is still the outside file, and asking the engine about that
   * path would keep answering null. */
  const [filed, setFiled] = createSignal<{ paper: Paper; inbox: string } | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [links, setLinks] = createSignal<CitationLinks | null>(null);
  const [linksError, setLinksError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<'bibtex' | 'lookup' | 'filing' | 'marking' | null>(null);
  const [copied, setCopied] = createSignal<'key' | 'bibtex' | null>(null);

  /** The entry once the engine has built it. The clipboard is only writable
   * for a moment after the click, and the first press spends that moment
   * fetching; a second press writes from here, inside its own gesture. */
  let bibtex: { id: string; text: string } | null = null;
  let paperSeq = 0;
  let linksSeq = 0;

  const shown = () => filed()?.paper ?? paper() ?? null;

  /** What the document says about being a paper, asked only once the
   * library has said it does not have it. SpeedyF is a viewer first: a
   * manual gets no offer, a paper gets one with the reason beside it. */
  const [paperness, setPaperness] = createSignal<Paperness | null>(null);
  const looksLikePaper = () => {
    const verdict = paperness()?.verdict;
    return verdict === 'paper' || verdict === 'likely';
  };
  /** What the scan took the document for, in the note's own words. */
  const looksLikeNoun = () => {
    const kind = paperness()?.kind;
    return kind === 'thesis' ? 'thesis' : kind === 'book' ? 'book' : 'paper';
  };

  const load = async (path: string) => {
    const seq = ++paperSeq;
    const result = await projectsStore.paperForPath(path);
    if (seq !== paperSeq) return;
    if (result === undefined) {
      setError(projectsStore.state.error ?? 'Could not read the library.');
      return;
    }
    rememberPaper(path, result);
    setPaper(result);
    if (result === null && doc.docId !== null) {
      const docId = doc.docId;
      engine
        .libraryClassifyDocument(docId)
        .then((verdict) => {
          if (seq === paperSeq) setPaperness(verdict);
        })
        .catch(() => {
          // Unknown is the quiet answer, and the right one when the engine
          // cannot read the page.
        });
    }
  };

  const loadLinks = (id: string) => {
    const seq = ++linksSeq;
    engine.libraryCitationLinks(id).then(
      (result) => {
        if (seq === linksSeq) setLinks(result);
      },
      (reason: unknown) => {
        if (seq !== linksSeq) return;
        const text = message(reason);
        if (!text.includes(NOT_IMPLEMENTED)) setLinksError(text);
      }
    );
  };

  const refreshFiled = async (copy: { paper: Paper; inbox: string }) => {
    try {
      const fresh = await engine.libraryPaper(copy.paper.id);
      if (filed()?.paper.id === copy.paper.id) setFiled({ paper: fresh, inbox: copy.inbox });
    } catch {
      // The last copy stays on screen; the next change event tries again.
    }
  };

  onMount(() => {
    void projectsStore.initialize();
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // An engine event, not a reactive scope: the reads here are of the
    // moment's state and must not subscribe to anything.
    const onChanged = () =>
      untrack(() => {
        forgetPapers();
        bibtex = null;
        if (doc.loaded && doc.path) void load(doc.path);
        const copy = filed();
        if (copy) void refreshFiled(copy);
        const id = shown()?.id;
        if (id) loadLinks(id);
      });
    void engine.onLibraryChanged(onChanged).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    onCleanup(() => {
      disposed = true;
      unlisten?.();
    });
  });

  createEffect(
    on(
      () => (doc.loaded ? doc.path : null),
      (path) => {
        setFiled(null);
        setError(null);
        setPaperness(null);
        bibtex = null;
        if (path) void load(path);
        else setPaper(undefined);
      }
    )
  );

  // A memo, so a refreshed record with the same id does not reset the lists
  // and refetch them a second time behind the refresh already under way.
  const shownId = createMemo(() => shown()?.id ?? null);
  createEffect(
    on(shownId, (id) => {
      linksSeq += 1;
      setLinks(null);
      setLinksError(null);
      if (id) loadLinks(id);
    })
  );

  const copyText = async (text: string, what: 'key' | 'bibtex'): Promise<boolean> => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(what);
      setTimeout(() => setCopied(null), 1600);
      return true;
    } catch {
      return false;
    }
  };

  const copyKey = async (current: Paper) => {
    setError(null);
    if (!(await copyText(current.citeKey, 'key'))) setError(CLIPBOARD_REFUSED);
  };

  const copyBibtex = async (current: Paper) => {
    setError(null);
    if (bibtex?.id === current.id) {
      if (!(await copyText(bibtex.text, 'bibtex'))) setError(CLIPBOARD_REFUSED);
      return;
    }
    setBusy('bibtex');
    const text = await projectsStore.bibtexFor([current.id]);
    setBusy(null);
    if (text === undefined) {
      setError(projectsStore.state.error ?? 'Could not build the BibTeX entry.');
      return;
    }
    bibtex = { id: current.id, text };
    if (!(await copyText(text, 'bibtex'))) setError(CLIPBOARD_REFUSED);
  };

  const lookUp = async (current: Paper) => {
    setBusy('lookup');
    setError(null);
    try {
      // 'applied' arrives as a change event, which re-reads the record.
      const result = await engine.libraryLookup(current.id, 'id');
      if (result.kind === 'nothing') setError('Nothing found for this identifier.');
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(null);
    }
  };

  /** The scan took this file for something other than a paper; the user
   * says otherwise. The change event re-reads the record, but the answer is
   * shown at once so the panel does not sit on a stale verdict meanwhile. */
  const markAsPaper = async (current: Paper) => {
    setBusy('marking');
    setError(null);
    const updated = await projectsStore.updatePaper(current.id, { isPaper: true });
    setBusy(null);
    if (!updated) {
      setError(projectsStore.state.error ?? 'Could not update the record.');
      return;
    }
    const copy = filed();
    if (copy && copy.paper.id === updated.id) {
      setFiled({ paper: updated, inbox: copy.inbox });
    } else {
      setPaper(updated);
      if (doc.path) rememberPaper(doc.path, updated);
    }
  };

  const setMember = async (current: Paper, projectId: string, member: boolean) => {
    setError(null);
    await projectsStore.setMembership(current.id, projectId, member);
    if (projectsStore.state.error) setError(projectsStore.state.error);
  };

  /** Copy the open file into the inbox folder and show the copy's record. The
   * inbox is chosen once and kept in settings (fileDrop.ts); a dismissed
   * dialog is a change of mind. */
  const addToLibrary = async () => {
    const path = doc.path;
    if (!path) return;
    const inbox = await chooseInbox();
    if (!inbox) return;
    setBusy('filing');
    setError(null);
    try {
      const copy = await engine.libraryFileIntoInbox(path, inbox);
      setFiled({ paper: copy, inbox });
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(null);
    }
  };

  const openRow = async (row: PaperRow) => {
    setError(null);
    try {
      const target = await engine.libraryPaper(row.id);
      const file = target.files.find((candidate) => !candidate.missing);
      if (!file) {
        setError('Every file of that item is missing.');
        return;
      }
      await openInNewTabOrFocus(file.path);
    } catch (reason) {
      setError(message(reason));
    }
  };

  /** The item's other files on disk, when the open document is one of
   * several: the review copy beside the preprint. */
  const otherFiles = (current: Paper) =>
    current.files.some((file) => file.path === doc.path)
      ? current.files.filter((file) => !file.missing && file.path !== doc.path)
      : [];
  const openOther = async (path: string) => {
    try {
      await openInNewTabOrFocus(path);
    } catch (reason) {
      setError(message(reason));
    }
  };

  const showInProjects = () => {
    updateSettings({ launchScreen: 'projects' });
    goHome();
  };

  const title = (current: Paper) =>
    current.meta.title?.value.trim() || current.files[0]?.name || 'Untitled';
  const yearVenue = (current: Paper) =>
    [current.meta.year?.value, current.meta.venue?.value].filter(Boolean).join(', ');
  const hasIdentifier = (current: Paper) => current.doi !== null || current.arxiv !== null;

  const LinkList = (props: { heading: string; rows: PaperRow[] }) => (
    <Show when={props.rows.length > 0}>
      <div class="paper-panel-subhead">{props.heading}</div>
      <For each={props.rows}>
        {(row) => (
          <button
            type="button"
            class="paper-panel-link-row"
            disabled={row.missing}
            title={row.missing ? 'Every file of this item is missing' : undefined}
            onClick={() => void openRow(row)}
          >
            <MathText class="paper-panel-link-title" text={row.title} />
            <span class="paper-panel-link-meta">
              {[row.authors, row.year].filter(Boolean).join(', ')}
            </span>
          </button>
        )}
      </For>
    </Show>
  );

  return (
    <div class="sidebar-scroll paper-panel" aria-label="Library">
      <Show
        when={doc.loaded}
        fallback={<div class="panel-note">Open a document to see its place in your library.</div>}
      >
        <Show
          when={shown()}
          fallback={
            <Show
              when={paper() !== undefined || error()}
              fallback={<div class="panel-note">Looking this document up in your library...</div>}
            >
              <Show when={paper() === null}>
                <Show
                  when={looksLikePaper()}
                  fallback={<div class="panel-note">This PDF is not in your library.</div>}
                >
                  <div class="panel-note">
                    This looks like a {looksLikeNoun()} ({paperness()!.reason}) and is not in your
                    library yet. Adding it copies the file into your inbox folder.
                  </div>
                </Show>
              </Show>
              <Show when={error()}>
                <div class="paper-panel-error" role="alert">
                  {error()}
                </div>
              </Show>
              <Show when={paper() === null}>
                <div class="paper-panel-actions">
                  <button
                    type="button"
                    class="paper-panel-btn"
                    classList={{
                      'primary-btn': looksLikePaper(),
                      'secondary-btn': !looksLikePaper(),
                    }}
                    disabled={busy() === 'filing'}
                    onClick={() => void addToLibrary()}
                  >
                    {busy() === 'filing' ? 'Copying...' : 'Add to library'}
                  </button>
                </div>
              </Show>
            </Show>
          }
        >
          {(current) => (
            <>
              <div class="paper-panel-head">
                <div class="paper-panel-title">
                  <MathText text={title(current())} />
                </div>
                <Show when={authorsLine(current().meta.authors?.value ?? [])}>
                  <div class="paper-panel-authors">
                    {authorsLine(current().meta.authors?.value ?? [])}
                  </div>
                </Show>
                <Show when={yearVenue(current())}>
                  <div class="paper-panel-meta">{yearVenue(current())}</div>
                </Show>
              </div>

              <Show when={otherFiles(current()).length > 0}>
                <div class="paper-panel-note paper-also">
                  <span>Also in this item: </span>
                  <For each={otherFiles(current())}>
                    {(file, index) => (
                      <>
                        {index() > 0 ? ', ' : ''}
                        <button
                          type="button"
                          class="paper-panel-link"
                          title={file.path}
                          onClick={() => void openOther(file.path)}
                        >
                          {file.name}
                        </button>
                      </>
                    )}
                  </For>
                </div>
              </Show>

              <Show when={filed()}>
                {(copy) => (
                  <div class="paper-panel-note">
                    <span>Copied into {copy().inbox}</span>
                    <button
                      type="button"
                      class="paper-panel-link"
                      onClick={() => void openInNewTabOrFocus(copy().paper.files[0]?.path ?? '')}
                      disabled={!copy().paper.files[0]}
                    >
                      Open the library copy
                    </button>
                  </div>
                )}
              </Show>

              {/* A record the scan did not take for a paper is in the
                  library, so it has a title and a place in Other PDFs, but
                  no cite key worth copying, no projects to tick and nothing
                  to look up — until the user says it is one. One quiet line
                  and the button; the reason is the scan's, in its words. */}
              <Show
                when={current().isPaper}
                fallback={
                  <>
                    <div class="paper-panel-note paper-panel-verdict">
                      <span>
                        Not treated as an item
                        {current().paperReason.trim() ? ` (${current().paperReason.trim()})` : ''}.
                      </span>
                      <button
                        type="button"
                        class="secondary-btn paper-panel-btn"
                        disabled={busy() === 'marking'}
                        onClick={() => void markAsPaper(current())}
                      >
                        {busy() === 'marking' ? 'Marking...' : 'Mark as item'}
                      </button>
                    </div>
                    <Show when={error()}>
                      <div class="paper-panel-error" role="alert">
                        {error()}
                      </div>
                    </Show>
                    <button type="button" class="paper-panel-link" onClick={showInProjects}>
                      Show in Projects
                    </button>
                  </>
                }
              >
                <div class="paper-panel-key-row">
                  <code class="paper-panel-key" title="Cite key">
                    {current().citeKey}
                  </code>
                  <button
                    type="button"
                    class="secondary-btn paper-panel-btn"
                    aria-label="Copy cite key"
                    onClick={() => void copyKey(current())}
                  >
                    {copied() === 'key' ? 'Copied' : 'Copy'}
                  </button>
                </div>

                <Show when={current().incomplete.length > 0}>
                  <div class="paper-panel-warn">
                    Missing for BibTeX: {current().incomplete.join(', ')}
                  </div>
                </Show>

                <div class="paper-panel-section">
                  <div class="paper-panel-section-title">Projects</div>
                  <Show
                    when={projectsStore.state.projects.length > 0}
                    fallback={<div class="paper-panel-dim">No projects yet.</div>}
                  >
                    <For each={projectsStore.state.projects}>
                      {(project) => (
                        <label class="paper-panel-check">
                          <input
                            type="checkbox"
                            checked={current().projectIds.includes(project.id)}
                            onInput={(e) =>
                              void setMember(current(), project.id, e.currentTarget.checked)
                            }
                          />
                          <span>{project.name}</span>
                        </label>
                      )}
                    </For>
                  </Show>
                  <ProjectSuggestionLine paperId={current().id} />
                </div>

                <div class="paper-panel-actions">
                  <button
                    type="button"
                    class="secondary-btn paper-panel-btn"
                    disabled={busy() === 'bibtex'}
                    onClick={() => void copyBibtex(current())}
                  >
                    {copied() === 'bibtex' ? 'Copied' : 'Copy BibTeX'}
                  </button>
                  <Show when={settings.onlineLookup}>
                    <button
                      type="button"
                      class="secondary-btn paper-panel-btn"
                      disabled={!hasIdentifier(current()) || busy() === 'lookup'}
                      title={hasIdentifier(current()) ? undefined : 'Needs a DOI or an arXiv id'}
                      onClick={() => void lookUp(current())}
                    >
                      {busy() === 'lookup' ? 'Looking up...' : 'Look up'}
                    </button>
                  </Show>
                </div>

                <Show when={error()}>
                  <div class="paper-panel-error" role="alert">
                    {error()}
                  </div>
                </Show>

                <button type="button" class="paper-panel-link" onClick={showInProjects}>
                  Show in Projects
                </button>

                <Show when={links() || linksError()}>
                  <div class="paper-panel-section">
                    <div class="paper-panel-section-title">In your library</div>
                    <Show when={linksError()}>
                      <div class="paper-panel-error">{linksError()}</div>
                    </Show>
                    <Show when={links()}>
                      {(graph) => (
                        <>
                          <Show when={graph().cites.length + graph().citedBy.length === 0}>
                            <div class="paper-panel-dim">
                              No links to other items in your library yet.
                            </div>
                          </Show>
                          <LinkList heading="Cites" rows={graph().cites} />
                          <LinkList heading="Cited by" rows={graph().citedBy} />
                          <RelatedItems
                            paperId={current().id}
                            onOpen={(row) => void openRow(row)}
                            frame={(rows) => (
                              <>
                                <div class="paper-panel-subhead">Related</div>
                                {rows}
                              </>
                            )}
                          />
                          <Show when={graph().unresolved > 0}>
                            <div class="paper-panel-dim">
                              {graph().unresolved} cited{' '}
                              {graph().unresolved === 1 ? 'identifier is' : 'identifiers are'} not
                              in your library.
                            </div>
                          </Show>
                        </>
                      )}
                    </Show>
                  </div>
                </Show>
              </Show>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
