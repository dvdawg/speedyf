/** The right column: one paper, everything known about it, each field
 * editable where it is shown.
 *
 * An edit sends only the field that changed. The engine records it as a user
 * value and never overwrites it afterwards, which is why every field wears
 * its provenance: "edited" is a promise, "first page" is a guess, and the
 * difference is what tells you whether a lookup is still worth running.
 *
 * The first page is rendered through the engine's library document pool the
 * way a citation hover is, so it costs what a hover preview costs, and it is
 * sequence-guarded the way the store's reads are: a slow preview for the
 * previous paper must not land on this one. */
import { createEffect, createSignal, createUniqueId, For, onCleanup, Show } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import { renderUrl } from '../../lib/rendering/renderSource';
import { pagesInverted, settings } from '../../stores/settings';
import type {
  LookupCandidate,
  LookupMode,
  MetaPatch,
  MetaSource,
  Paper,
  PaperKind,
  ResolvedCitation,
} from '../../types/engine';
import { DuplicateLine, MergedLine } from './MergeConfirm';
import { MissingWorkPanel } from './MissingWorks';
import { openPaper } from './openPaper';
import {
  authorsLine,
  formatAuthors,
  KINDS,
  parseAuthors,
  parseYear,
  SOURCE_LABEL,
} from './paperFields';
import { projectsStore } from './projectsStore';

/** Text fields that save as they are typed, trimmed. Year, authors, kind and
 * the abstract each need something more and are wired by hand below. */
const TEXT_FIELDS = [
  { key: 'title', label: 'Title' },
  { key: 'venue', label: 'Venue' },
  { key: 'volume', label: 'Volume' },
  { key: 'issue', label: 'Issue' },
  { key: 'pages', label: 'Pages' },
  { key: 'publisher', label: 'Publisher' },
] as const;
type TextKey = (typeof TEXT_FIELDS)[number]['key'];

function textPatch(key: TextKey | 'doi' | 'arxiv' | 'isbn', value: string | null): MetaPatch {
  const patch: MetaPatch = {};
  patch[key] = value;
  return patch;
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

function focusLater(el: HTMLElement) {
  // The ref fires before the element is in the document.
  queueMicrotask(() => {
    el.focus();
    if (el instanceof HTMLInputElement) el.select();
  });
}

/** "Given Family · 1999 · Venue", for a candidate's one-line summary. */
function candidateMeta(candidate: LookupCandidate): string {
  return [authorsLine(candidate.authors), candidate.year, candidate.venue]
    .filter((part) => part !== null && part !== '')
    .join(' · ');
}

function SourceTag(props: { source: MetaSource | null | undefined }) {
  return (
    <Show when={props.source}>
      {(source) => <span class="paper-field-source">{SOURCE_LABEL[source()]}</span>}
    </Show>
  );
}

/** One field, shown as text until clicked (or reached and Entered), then as
 * an input. Enter saves, Escape cancels, and leaving the field saves too,
 * because a value typed and then clicked away from was meant. A multi-line
 * field saves on cmd/ctrl+Enter, since Enter is how its lines are made.
 *
 * `onSave` returns a message to keep the field open with that problem, or
 * null once the value went through. */
function EditableField(props: {
  label: string;
  display: string;
  text: string;
  source?: MetaSource | null | undefined;
  multiline?: boolean;
  /** Shown folded until asked for; the abstract. */
  collapsible?: boolean;
  mono?: boolean;
  onSave: (text: string) => Promise<string | null>;
}) {
  const id = createUniqueId();
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal('');
  const [problem, setProblem] = createSignal<string | null>(null);
  const [expanded, setExpanded] = createSignal(false);
  // Enter and the blur that removing the editor causes must not save twice;
  // Escape and that same blur must not save at all.
  let done = false;

  const begin = () => {
    setDraft(props.text);
    setProblem(null);
    done = false;
    setEditing(true);
  };
  const cancel = () => {
    if (done) return;
    done = true;
    setEditing(false);
  };
  const commit = async () => {
    if (done) return;
    done = true;
    if (draft() === props.text) {
      setEditing(false);
      return;
    }
    const message = await props.onSave(draft());
    if (message) {
      done = false;
      setProblem(message);
      return;
    }
    setEditing(false);
  };
  const keys = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    } else if (e.key === 'Enter' && (!props.multiline || e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopPropagation();
      void commit();
    }
  };

  return (
    <div class="paper-field" classList={{ 'is-multiline': props.multiline === true }}>
      <span class="paper-field-label" id={id}>
        {props.label}
      </span>
      <div class="paper-field-body">
        <Show
          when={editing()}
          fallback={
            <Show
              when={!props.collapsible || expanded() || !props.display}
              fallback={
                <button
                  type="button"
                  class="paper-field-value is-dim"
                  aria-labelledby={id}
                  onClick={() => setExpanded(true)}
                >
                  Show
                </button>
              }
            >
              <button
                type="button"
                class="paper-field-value"
                classList={{ 'is-empty': !props.display, 'is-mono': props.mono === true }}
                aria-labelledby={id}
                onClick={begin}
              >
                {props.display || 'Add'}
              </button>
            </Show>
          }
        >
          <Show
            when={props.multiline}
            fallback={
              <input
                type="text"
                class="paper-field-input"
                classList={{ 'is-mono': props.mono === true }}
                aria-labelledby={id}
                aria-invalid={problem() !== null}
                value={draft()}
                ref={focusLater}
                onInput={(e) => setDraft(e.currentTarget.value)}
                onKeyDown={keys}
                onBlur={() => void commit()}
              />
            }
          >
            <textarea
              class="paper-field-input"
              aria-labelledby={id}
              rows={props.collapsible ? 6 : 3}
              value={draft()}
              ref={focusLater}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={keys}
              onBlur={() => void commit()}
            />
          </Show>
        </Show>
        <SourceTag source={props.source} />
        <Show when={problem()}>
          <span class="paper-inline-error" role="alert">
            {problem()}
          </span>
        </Show>
      </div>
    </div>
  );
}

function Preview(props: { paperId: string | null }) {
  const [resolved, setResolved] = createSignal<ResolvedCitation | null>(null);
  const [loading, setLoading] = createSignal(false);
  let seq = 0;

  createEffect(() => {
    const id = props.paperId;
    const mine = ++seq;
    setResolved(null);
    if (!id) {
      setLoading(false);
      return;
    }
    setLoading(true);
    engine
      .libraryPreview(id)
      .then((result) => {
        if (mine !== seq) return;
        setResolved(result);
        setLoading(false);
      })
      .catch(() => {
        // A paper with no readable file simply has no first page to show;
        // the files list below says why.
        if (mine !== seq) return;
        setResolved(null);
        setLoading(false);
      });
  });

  const url = (r: ResolvedCitation) =>
    renderUrl({
      docId: r.preview.docId,
      srcIndex: r.preview.src,
      rotation: 0,
      scaleMilli: r.preview.scaleMilli,
      // A library document is never edited, so its generation is always 0.
      generation: 0,
      kind: 'preview',
      tile: r.preview.tile,
      invert: pagesInverted(),
    });

  return (
    <Show when={loading() || resolved()}>
      <div class="paper-preview">
        <Show when={loading()}>
          <div
            class="citation-preview-skeleton paper-preview-skeleton"
            aria-label="Loading first page"
          />
        </Show>
        <Show when={resolved()}>
          {(r) => (
            <img
              class="paper-preview-image"
              src={url(r())}
              alt={`First page of ${r().title ?? r().fileName}`}
              width={r().preview.tile.w}
              height={r().preview.tile.h}
              style={{ 'aspect-ratio': `${r().preview.tile.w} / ${r().preview.tile.h}` }}
              draggable={false}
              onError={() => setResolved(null)}
            />
          )}
        </Show>
      </div>
    </Show>
  );
}

/** "Looks like a thesis: …", "Marked as an item by you", "Not treated as an
 * item: …". The scan's kind names what it took the file for; a record it
 * passed over, or the user overruled, has no kind worth naming. */
function verdictText(paper: Paper): string {
  const reason = paper.paperReason.trim();
  if (paper.paperOverride !== null) {
    return paper.isPaper ? 'Marked as an item by you' : 'Marked as not an item by you';
  }
  if (!paper.isPaper) {
    return `Not treated as an item: ${reason || 'no signs of a paper, thesis or book'}`;
  }
  const kind = paper.meta.kind?.value;
  const noun = kind === 'thesis' ? 'thesis' : kind === 'book' ? 'book' : 'paper';
  return reason ? `Looks like a ${noun}: ${reason}` : `Looks like a ${noun}`;
}

/** Why this record is, or is not, treated as an item, and the way to say
 * otherwise. The verdict decides what All items and Not filed list and what
 * a lookup bothers with, and a library folder like Downloads holds more
 * manuals and forms than papers, so the scan's reason is shown rather than
 * trusted: a form that slipped through, or a paper the heuristics missed, is
 * one click from fixed and one more from handed back to the scan. */
function Verdict(props: { paper: Paper }) {
  const store = projectsStore;
  const set = (isPaper: boolean | null) => void store.updatePaper(props.paper.id, { isPaper });
  return (
    <div class="paper-verdict" classList={{ 'is-other': !props.paper.isPaper }}>
      <span class="paper-verdict-text">{verdictText(props.paper)}</span>
      <Show
        when={props.paper.isPaper}
        fallback={
          <button
            type="button"
            class="paper-mini-btn"
            title="List it with the items and give it a cite key"
            onClick={() => set(true)}
          >
            It is an item
          </button>
        }
      >
        <button
          type="button"
          class="paper-mini-btn"
          title="Move it to Other PDFs; lookups will leave it alone"
          onClick={() => set(false)}
        >
          Not an item
        </button>
      </Show>
      <Show when={props.paper.paperOverride !== null}>
        <button
          type="button"
          class="paper-verdict-link"
          title="Clear your mark and go back to the scan’s verdict"
          onClick={() => set(null)}
        >
          Let the scan decide
        </button>
      </Show>
    </div>
  );
}

function Fields(props: { paper: Paper }) {
  const store = projectsStore;
  const save = async (patch: MetaPatch): Promise<string | null> => {
    await store.updatePaper(props.paper.id, patch);
    return null;
  };
  const text = (key: TextKey) => props.paper.meta[key]?.value ?? '';
  const authors = () => props.paper.meta.authors?.value ?? [];

  return (
    <section class="paper-fields" aria-label="Metadata">
      <EditableField
        label="Title"
        display={text('title')}
        text={text('title')}
        source={props.paper.meta.title?.source}
        onSave={(v) => save(textPatch('title', v.trim() || null))}
      />
      <EditableField
        label="Authors"
        display={formatAuthors(authors())}
        text={formatAuthors(authors())}
        source={props.paper.meta.authors?.source}
        multiline
        onSave={(v) => {
          const list = parseAuthors(v);
          return save({ authors: list.length > 0 ? list : null });
        }}
      />
      <EditableField
        label="Year"
        display={props.paper.meta.year ? String(props.paper.meta.year.value) : ''}
        text={props.paper.meta.year ? String(props.paper.meta.year.value) : ''}
        source={props.paper.meta.year?.source}
        onSave={(v) => {
          const year = parseYear(v);
          if (year === 'invalid') return Promise.resolve('A year is up to four digits');
          return save({ year });
        }}
      />
      <For each={TEXT_FIELDS.filter((f) => f.key !== 'title')}>
        {(field) => (
          <EditableField
            label={field.label}
            display={text(field.key)}
            text={text(field.key)}
            source={props.paper.meta[field.key]?.source}
            onSave={(v) => save(textPatch(field.key, v.trim() || null))}
          />
        )}
      </For>
      <div class="paper-field">
        <label class="paper-field-label" for={`paper-kind-${props.paper.id}`}>
          Kind
        </label>
        <div class="paper-field-body">
          <select
            id={`paper-kind-${props.paper.id}`}
            class="paper-field-select"
            value={props.paper.meta.kind?.value ?? ''}
            onChange={(e) => {
              const value = e.currentTarget.value;
              void save({ kind: value ? (value as PaperKind) : null });
            }}
          >
            <option value="">—</option>
            <For each={KINDS}>{(kind) => <option value={kind}>{kind}</option>}</For>
          </select>
          <SourceTag source={props.paper.meta.kind?.source} />
        </div>
      </div>
      <EditableField
        label="Abstract"
        display={props.paper.meta.abstract?.value ?? ''}
        text={props.paper.meta.abstract?.value ?? ''}
        source={props.paper.meta.abstract?.source}
        multiline
        collapsible
        onSave={(v) => save({ abstract: v.trim() || null })}
      />
      <EditableField
        label="DOI"
        display={props.paper.doi ?? ''}
        text={props.paper.doi ?? ''}
        mono
        onSave={(v) => save(textPatch('doi', v.trim() || null))}
      />
      <EditableField
        label="arXiv"
        display={props.paper.arxiv ?? ''}
        text={props.paper.arxiv ?? ''}
        mono
        onSave={(v) => save(textPatch('arxiv', v.trim() || null))}
      />
      <EditableField
        label="ISBN"
        display={props.paper.isbn ?? ''}
        text={props.paper.isbn ?? ''}
        mono
        onSave={(v) => save(textPatch('isbn', v.trim() || null))}
      />
    </section>
  );
}

function CiteKey(props: { paper: Paper; copied: boolean; onCopy: () => void }) {
  const store = projectsStore;
  const [editing, setEditing] = createSignal(false);
  let done = false;
  const finish = (value: string | null) => {
    if (done) return;
    done = true;
    setEditing(false);
    const key = value?.trim();
    if (key && key !== props.paper.citeKey)
      void store.updatePaper(props.paper.id, { citeKey: key });
  };
  return (
    <div class="paper-citekey">
      <span class="paper-field-label">Cite key</span>
      <div class="paper-field-body paper-citekey-body">
        <Show
          when={editing()}
          fallback={
            <>
              <code class="paper-citekey-value">{props.paper.citeKey}</code>
              <Show when={props.paper.citeKeyPinned}>
                <span class="paper-field-source">pinned</span>
              </Show>
              <button type="button" class="paper-mini-btn" onClick={() => props.onCopy()}>
                {props.copied ? 'Copied' : 'Copy'}
              </button>
              <button
                type="button"
                class="paper-mini-btn"
                onClick={() => {
                  done = false;
                  setEditing(true);
                }}
              >
                edit
              </button>
              <Show when={props.paper.citeKeyPinned}>
                <button
                  type="button"
                  class="paper-mini-btn"
                  title="Unpin and regenerate from the metadata"
                  onClick={() => void store.updatePaper(props.paper.id, { citeKey: null })}
                >
                  reset
                </button>
              </Show>
            </>
          }
        >
          <input
            type="text"
            class="paper-field-input is-mono"
            aria-label="Cite key"
            value={props.paper.citeKey}
            ref={focusLater}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                e.stopPropagation();
                finish(e.currentTarget.value);
              } else if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                finish(null);
              }
            }}
            onBlur={(e) => finish(e.currentTarget.value)}
          />
        </Show>
      </div>
    </div>
  );
}

export default function PaperDetail(props: { onMoveFocus: (dir: -1 | 1) => void }) {
  const store = projectsStore;
  const paper = () => (ghost() ? null : store.state.selected);
  /** A missing work picked in the graph stands in for the record. */
  const ghost = () => {
    const key = store.state.ghostKey;
    return key ? (store.state.missingWorks.find((w) => w.key === key) ?? null) : null;
  };

  const [copied, setCopied] = createSignal<'key' | 'bibtex' | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  const flash = (what: 'key' | 'bibtex') => {
    setCopied(what);
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => setCopied(null), 1_200);
  };
  onCleanup(() => clearTimeout(copiedTimer));

  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<{ text: string; error: boolean } | null>(null);
  const [candidates, setCandidates] = createSignal<LookupCandidate[] | null>(null);

  // Whatever a lookup left behind belongs to the paper it ran for.
  createEffect(() => {
    void store.state.selectedId;
    setNotice(null);
    setCandidates(null);
  });

  const copyKey = async (p: Paper) => {
    if (await copyText(p.citeKey)) flash('key');
    else setNotice({ text: 'Could not write to the clipboard', error: true });
  };

  // A clipboard write has to land inside the user gesture, and the engine
  // round trip may outlast WebKit's window for that. The entry is kept for
  // the paper it was rendered for, so a second press writes at once.
  let bibCache: { id: string; updatedAt: string; text: string } | null = null;
  const copyBibtex = async (p: Paper) => {
    let bib =
      bibCache && bibCache.id === p.id && bibCache.updatedAt === p.updatedAt
        ? bibCache.text
        : undefined;
    if (bib === undefined) {
      bib = await store.bibtexFor([p.id]);
      if (bib === undefined) return;
      bibCache = { id: p.id, updatedAt: p.updatedAt, text: bib };
    }
    if (await copyText(bib)) flash('bibtex');
    else setNotice({ text: 'Could not write to the clipboard — press Copy again', error: true });
  };

  const lookup = async (p: Paper, mode: LookupMode) => {
    setBusy(true);
    setNotice(null);
    setCandidates(null);
    try {
      const result = await engine.libraryLookup(p.id, mode);
      if (store.state.selectedId !== p.id) return;
      if (result.kind === 'applied') {
        void store.loadDetail(p.id);
        setNotice({ text: 'Metadata updated', error: false });
      } else if (result.kind === 'candidates') {
        setCandidates(result.candidates);
      } else {
        setNotice({ text: 'Nothing found', error: false });
      }
    } catch (error) {
      // In the bar at the top like every other failure, and here beside the
      // button that was pressed, so the two are not hunted for separately.
      setNotice({ text: store.reportError(error), error: true });
    } finally {
      setBusy(false);
    }
  };

  /** One pick, whether from a title search just run or from the matches a
   * batch lookup parked on the paper. The engine applies it and drops the
   * rest; the refreshed record says so. */
  const apply = async (p: Paper, candidate: LookupCandidate) => {
    setBusy(true);
    try {
      await engine.libraryApplyCandidate(p.id, candidate);
      if (store.state.selectedId !== p.id) return;
      setCandidates(null);
      void store.loadDetail(p.id);
    } catch (error) {
      setNotice({ text: store.reportError(error), error: true });
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowLeft') return;
    const target = e.target as HTMLElement;
    const typing =
      (target.tagName === 'INPUT' && target.getAttribute('type') !== 'checkbox') ||
      target.tagName === 'TEXTAREA' ||
      target.tagName === 'SELECT';
    if (typing) return;
    e.preventDefault();
    props.onMoveFocus(-1);
  };

  return (
    <div class="projects-col-scroll paper-detail" onKeyDown={onKeyDown}>
      <Show when={ghost()}>{(work) => <MissingWorkPanel work={work()} />}</Show>
      <Show when={paper()}>
        {(p) => (
          <>
            <Preview paperId={p().id} />
            <Verdict paper={p()} />
            <DuplicateLine paper={p()} />
            <MergedLine paper={p()} />
            <Show when={p().candidates.length > 0}>
              <div class="paper-parked" role="group" aria-label="Possible matches">
                <div class="paper-candidates-head">
                  <span>Possible matches ({p().candidates.length})</span>
                  <button
                    type="button"
                    class="paper-mini-btn"
                    disabled={busy()}
                    title="Forget these; a later Find by title can ask again"
                    onClick={() => void store.discardCandidates(p().id)}
                  >
                    Dismiss all
                  </button>
                </div>
                <For each={p().candidates}>
                  {(candidate) => (
                    <div class="paper-parked-item">
                      <span class="paper-candidate-title">{candidate.title}</span>
                      <Show when={candidateMeta(candidate)}>
                        <span class="paper-candidate-meta">{candidateMeta(candidate)}</span>
                      </Show>
                      <span class="paper-parked-foot">
                        <span class="paper-field-source">{SOURCE_LABEL[candidate.source]}</span>
                        <button
                          type="button"
                          class="paper-mini-btn"
                          disabled={busy()}
                          onClick={() => void apply(p(), candidate)}
                        >
                          Use this
                        </button>
                      </span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
            <Fields paper={p()} />
            {/* A cite key, a BibTeX entry and a lookup are things a paper has.
                A manual filed into a project keeps its files, its projects
                and Open, and gets the rest back the moment it is marked one. */}
            <Show when={p().isPaper}>
              <CiteKey paper={p()} copied={copied() === 'key'} onCopy={() => void copyKey(p())} />
            </Show>

            <div class="paper-section">
              <span class="paper-field-label">Files</span>
              <ul class="paper-files">
                <For each={p().files}>
                  {(file) => (
                    <li
                      class="paper-file"
                      classList={{ 'is-missing': file.missing }}
                      title={file.path}
                    >
                      <span class="paper-file-name">{file.name}</span>
                      <Show when={file.missing}>
                        <span class="paper-badge">missing</span>
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </div>

            <div class="paper-section">
              <span class="paper-field-label">Projects</span>
              <Show
                when={store.state.projects.length > 0}
                fallback={<span class="paper-dim">No projects yet</span>}
              >
                <div class="paper-projects">
                  <For each={store.state.projects}>
                    {(project) => (
                      <label class="paper-project-check">
                        <input
                          type="checkbox"
                          checked={p().projectIds.includes(project.id)}
                          onChange={(e) =>
                            void store.setMembership(p().id, project.id, e.currentTarget.checked)
                          }
                        />
                        <span>{project.name}</span>
                      </label>
                    )}
                  </For>
                </div>
              </Show>
            </div>

            <Show when={p().isPaper && p().incomplete.length > 0}>
              <p class="paper-incomplete sb-warn">
                Missing for BibTeX: {p().incomplete.join(', ')}
              </p>
            </Show>

            <div class="paper-actions">
              <button type="button" class="primary-btn" onClick={() => void openPaper(p().id)}>
                Open
              </button>
              <Show when={p().isPaper}>
                <button type="button" class="secondary-btn" onClick={() => void copyBibtex(p())}>
                  {copied() === 'bibtex' ? 'Copied' : 'Copy BibTeX'}
                </button>
              </Show>
              <Show when={settings.onlineLookup && p().isPaper}>
                <button
                  type="button"
                  class="secondary-btn"
                  disabled={busy() || (!p().doi && !p().arxiv)}
                  title="Fetch metadata by DOI or arXiv id"
                  onClick={() => void lookup(p(), 'id')}
                >
                  Look up
                </button>
                <button
                  type="button"
                  class="secondary-btn"
                  disabled={busy()}
                  title="Search by title and pick a match"
                  onClick={() => void lookup(p(), 'title')}
                >
                  Find by title
                </button>
              </Show>
              <button
                type="button"
                class="secondary-btn"
                title="The citation graph around this item"
                onClick={() => store.showInGraph(p().id)}
              >
                Show in graph
              </button>
            </div>
            <Show when={notice()}>
              {(n) => (
                <p
                  class="paper-notice"
                  classList={{ 'paper-inline-error': n().error }}
                  role={n().error ? 'alert' : 'status'}
                >
                  {n().text}
                </p>
              )}
            </Show>
            <Show when={candidates()}>
              {(list) => (
                <div class="paper-candidates" role="group" aria-label="Lookup candidates">
                  <div class="paper-candidates-head">
                    <span>{list().length > 0 ? 'Which is it?' : 'Nothing found'}</span>
                    <button
                      type="button"
                      class="paper-mini-btn"
                      onClick={() => setCandidates(null)}
                    >
                      Cancel
                    </button>
                  </div>
                  <For each={list()}>
                    {(candidate) => (
                      <button
                        type="button"
                        class="paper-candidate"
                        disabled={busy()}
                        onClick={() => void apply(p(), candidate)}
                      >
                        <span class="paper-candidate-title">{candidate.title}</span>
                        <span class="paper-candidate-meta">{candidateMeta(candidate)}</span>
                        <span class="paper-field-source">{SOURCE_LABEL[candidate.source]}</span>
                      </button>
                    )}
                  </For>
                </div>
              )}
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
