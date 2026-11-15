/** The review drawer on the right of the reader, while reviewing: the
 * Overall paragraph, the comments grouped by section in reading order
 * (filterable by kind), the papers the reviewer may want to suggest, and
 * the two ways to send it all back - Export PDF... and Copy as text.
 *
 * It takes the place of an extension's drawer (one drawer at a time), at
 * the same width, resized the same way. */
import { createMemo, createSignal, For, onMount, Show } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconChevronDown, IconChevronRight, IconClose, IconRegion } from '../../components/icons';
import { formatShortcut } from '../../app/shortcutLabel';
import { activeTab } from '../../stores/tabsStore';
import { paneWidth, setPaneWidth, shownPane } from '../extensions/surfaces';
import { readerItem } from '../extensions/readerContext';
import CommentCard from './CommentCard';
import type { EditorResult } from './CommentEditor';
import { copyMarkupText, exportMarkupPdf } from './exportActions';
import { buildEntries, groupBySection, kindCounts, summarySkeleton } from './model';
import { labelFor } from './labels';
import { loadMath } from './mathHtml';
import {
  activePath,
  beginCommentAt,
  beginRegion,
  cancelDraft,
  commitDraft,
  deleteComment,
  draft,
  editComment,
  editing,
  flashCard,
  kindFilter,
  labelContext,
  reviewFor,
  reviewing,
  revealComment,
  setKindFilter,
  setSummary,
  stopReviewing,
  updateComment,
} from './markupStore';
import { loadSuggestions, referencesAnchor, type Suggestion } from './suggestions';
import { COMMENT_KINDS, KIND_LABEL, type MarkupComment } from './types';

export default function MarkupDrawer() {
  return (
    <Show when={reviewing() && !shownPane() && activePath()}>
      {(path) => <Drawer path={path()} />}
    </Show>
  );
}

function Drawer(props: { path: string }) {
  void loadMath();
  const review = () => reviewFor(props.path);
  const comments = () => review()?.doc.comments ?? [];

  /** Every comment, and the one being written, numbered and labelled. */
  const entries = createMemo(() => {
    const d = draft();
    const all: MarkupComment[] = [...comments()];
    if (d && d.path === props.path) {
      all.push({
        id: 'draft',
        kind: d.kind,
        anchor: d.anchor,
        body: d.body ?? '',
        replacement: d.mode === 'typo' ? '' : null,
        quickmark: d.quickmark ?? null,
        created: Number.MAX_SAFE_INTEGER,
      });
    }
    const ctx = labelContext(all);
    return buildEntries(all, (anchor) => labelFor(anchor, ctx));
  });

  const byId = createMemo(() => new Map(entries().map((e) => [e.comment.id, e])));
  const counts = createMemo(() => kindCounts(comments()));
  const groups = createMemo(() => {
    const filter = kindFilter();
    const shown = entries().filter(
      (e) => e.comment.id === 'draft' || filter === null || e.comment.kind === filter
    );
    return groupBySection(shown);
  });

  const save = (id: string, r: EditorResult) => {
    if (id === 'draft') {
      const saved = commitDraft(r);
      if (saved) flashCard(saved);
      return;
    }
    updateComment(props.path, id, {
      body: r.body.trim(),
      kind: r.kind,
      quickmark: r.quickmark,
      ...(r.replacement !== null ? { replacement: r.replacement.trim() } : {}),
    });
    editComment(null);
  };

  let aside!: HTMLElement;
  const startResize = (e: PointerEvent) => {
    e.preventDefault();
    const right = aside.getBoundingClientRect().right;
    const move = (ev: PointerEvent) => setPaneWidth(right - ev.clientX);
    const finish = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      document.body.classList.remove('is-resizing-sidebar');
    };
    document.body.classList.add('is-resizing-sidebar');
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
  };

  return (
    <aside
      ref={aside}
      class="reader-pane mk-drawer"
      style={{ width: `${paneWidth()}px` }}
      aria-label="Review"
    >
      <div
        class="reader-pane-resize"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize review"
        tabindex="0"
        onPointerDown={startResize}
        onKeyDown={(e) => {
          const step = e.shiftKey ? 40 : 10;
          if (e.key === 'ArrowLeft') setPaneWidth(paneWidth() + step);
          else if (e.key === 'ArrowRight') setPaneWidth(paneWidth() - step);
          else return;
          e.preventDefault();
        }}
      />
      <header class="mk-head">
        <span class="reader-pane-title">Review</span>
        <span class="mk-head-count">
          {comments().length === 1 ? '1 comment' : `${comments().length} comments`}
        </span>
        <span class="mk-head-tools">
          <IconButton label="Comment on a region (R)" onClick={() => void beginRegion()}>
            <IconRegion />
          </IconButton>
          <button
            type="button"
            class="reader-pane-close"
            aria-label={`Stop reviewing (${formatShortcut('mod+shift+m')})`}
            title={`Stop reviewing (${formatShortcut('mod+shift+m')})`}
            onClick={() => stopReviewing()}
          >
            <IconClose />
          </button>
        </span>
      </header>

      <div class="mk-scroll">
        <Show
          when={review()?.status !== 'failed'}
          fallback={
            <p class="mk-note-line">
              The saved review of this PDF could not be read, so nothing is written over it.
            </p>
          }
        >
          <Overall
            path={props.path}
            summary={review()?.doc.summary ?? ''}
            comments={comments()}
            ready={review()?.status === 'ready'}
          />

          <Show when={comments().length > 0}>
            <div class="mk-filter" role="group" aria-label="Show comments of kind">
              <button
                type="button"
                class="mk-filter-chip"
                classList={{ 'is-active': kindFilter() === null }}
                aria-pressed={kindFilter() === null}
                onClick={() => setKindFilter(null)}
              >
                All <span class="mk-filter-n">{comments().length}</span>
              </button>
              <For each={COMMENT_KINDS}>
                {(kind) => (
                  <button
                    type="button"
                    class={`mk-filter-chip is-${kind}`}
                    classList={{ 'is-active': kindFilter() === kind }}
                    aria-pressed={kindFilter() === kind}
                    disabled={counts()[kind] === 0}
                    onClick={() => setKindFilter(kindFilter() === kind ? null : kind)}
                  >
                    {KIND_LABEL[kind]} <span class="mk-filter-n">{counts()[kind]}</span>
                  </button>
                )}
              </For>
            </div>
          </Show>

          <Show
            when={entries().length > 0}
            fallback={
              <div class="mk-empty">
                <p>Select text on the page, then:</p>
                <ul>
                  <li>
                    <kbd>C</kbd> to comment on it
                  </li>
                  <li>
                    <kbd>T</kbd> to suggest a fix for a typo or wording
                  </li>
                  <li>
                    <kbd>R</kbd> marks a region instead: a figure, an equation
                  </li>
                </ul>
                <p class="mk-dim">
                  Comments are numbered, labelled and grouped by section as you go. Type / in a
                  comment for QuickMarks.
                </p>
              </div>
            }
          >
            {/* Keyed by id, not by object: the lists are rebuilt whenever a
                label arrives, and a card being written in must survive that. */}
            <For each={groups().map((g) => g.key)}>
              {(key) => {
                const group = () => groups().find((g) => g.key === key);
                return (
                  <Show when={group()}>
                    {(g) => (
                      <section class="mk-group">
                        <h3 class="mk-group-title" title={g().title}>
                          {g().title}
                        </h3>
                        <For each={g().entries.map((e) => e.comment.id)}>
                          {(id) => (
                            <Show when={byId().get(id)}>
                              {(entry) => (
                                <CommentCard
                                  entry={entry()}
                                  draft={id === 'draft'}
                                  editing={id === 'draft' || editing() === id}
                                  onOpen={() => {
                                    if (id !== 'draft') revealComment(entry().comment);
                                  }}
                                  onEdit={() => editComment(id)}
                                  onDelete={() => deleteComment(props.path, id)}
                                  onKind={(kind) => updateComment(props.path, id, { kind })}
                                  onSave={(r) => save(id, r)}
                                  onCancel={() =>
                                    id === 'draft' ? cancelDraft() : editComment(null)
                                  }
                                />
                              )}
                            </Show>
                          )}
                        </For>
                      </section>
                    )}
                  </Show>
                );
              }}
            </For>
          </Show>

          <Suggestions path={props.path} />
        </Show>
      </div>

      <footer class="mk-foot">
        <button
          type="button"
          class="primary-btn"
          title="The paper with your comments marked and their notes in a margin beside each page"
          onClick={() => void exportMarkupPdf()}
        >
          Export PDF...
        </button>
        <button
          type="button"
          class="secondary-btn"
          title="The review as a plain letter, grouped by section, for an email"
          onClick={() => void copyMarkupText()}
        >
          Copy as text
        </button>
      </footer>
    </aside>
  );
}

/** The Overall paragraph: what the reviewer thinks of the paper as a whole.
 * Offered a starting point with the counts filled in. */
function Overall(props: {
  path: string;
  summary: string;
  comments: readonly MarkupComment[];
  ready: boolean;
}) {
  let field!: HTMLTextAreaElement;
  const grow = () => {
    field.style.height = 'auto';
    field.style.height = `${Math.max(field.scrollHeight, 44)}px`;
  };
  onMount(grow);
  const start = () => {
    const text = summarySkeleton(props.comments);
    setSummary(props.path, text);
    field.value = text;
    grow();
    field.focus();
    const at = text.indexOf('\n');
    field.setSelectionRange(at < 0 ? text.length : at, at < 0 ? text.length : at);
  };
  return (
    <div class="mk-overall">
      <div class="mk-overall-head">
        <label class="mk-section-label" for="mk-overall-field">
          Overall
        </label>
        <Show when={!props.summary.trim() && props.ready}>
          <button type="button" class="mk-text-btn" onClick={start}>
            Start from an outline
          </button>
        </Show>
      </div>
      <textarea
        ref={field}
        id="mk-overall-field"
        class="mk-overall-field"
        value={props.summary}
        disabled={!props.ready}
        placeholder="What you think of the paper as a whole. Markdown, $math$ and @ citations work here too."
        onInput={(e) => {
          setSummary(props.path, e.currentTarget.value);
          grow();
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === 'Escape') e.currentTarget.blur();
        }}
      />
    </div>
  );
}

/** Papers in the library the citation graph ties to this one that it does
 * not cite. Folded away until asked for. */
function Suggestions(props: { path: string }) {
  const [open, setOpen] = createSignal(false);
  const [rows, setRows] = createSignal<Suggestion[] | null>(null);
  const [failed, setFailed] = createSignal(false);
  const item = () => readerItem();

  const toggle = () => {
    const next = !open();
    setOpen(next);
    const it = item();
    if (next && it && rows() === null) {
      loadSuggestions(it.id).then(setRows, () => setFailed(true));
    }
  };

  const suggest = async (s: Suggestion) => {
    const tab = activeTab();
    if (!tab) return;
    const anchor = await referencesAnchor(tab);
    if (!anchor) return;
    beginCommentAt(props.path, anchor, {
      kind: 'literature',
      body: `Related work to consider: [@${s.key}]. `,
      quickmark: 'related',
    });
  };

  return (
    <section class="mk-suggest">
      <button type="button" class="mk-suggest-toggle" aria-expanded={open()} onClick={toggle}>
        <Show when={open()} fallback={<IconChevronRight />}>
          <IconChevronDown />
        </Show>
        Papers you may have missed
      </button>
      <Show when={open()}>
        <Show
          when={item()}
          fallback={
            <p class="mk-note-line">
              Suggestions come from your library's citation graph. They appear once this PDF is an
              item of the library.
            </p>
          }
        >
          <Show
            when={rows()}
            fallback={
              <p class="mk-note-line">
                {failed() ? 'Suggestions could not be read.' : 'Reading the citation graph...'}
              </p>
            }
          >
            {(list) => (
              <Show
                when={list().length > 0}
                fallback={
                  <p class="mk-note-line">
                    Nothing in your library looks missing from its references.
                  </p>
                }
              >
                <For each={list()}>
                  {(s) => (
                    <div class="mk-suggest-row">
                      <div class="mk-suggest-text">
                        <div class="mk-suggest-title">{s.title || s.key}</div>
                        <div class="mk-suggest-detail">{s.detail}</div>
                        <div class="mk-suggest-detail">{s.reason}</div>
                      </div>
                      <button
                        type="button"
                        class="mk-text-btn"
                        title="Add a literature comment citing it"
                        onClick={() => void suggest(s)}
                      >
                        Suggest
                      </button>
                    </div>
                  )}
                </For>
              </Show>
            )}
          </Show>
        </Show>
      </Show>
    </section>
  );
}
