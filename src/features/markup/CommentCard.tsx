/** One comment in the drawer: its number, where it is ("p. 5 . Sec. 2.1 .
 * Lemma 2.3"), its kind, the words it is about and what it says. A click
 * goes to it on the page; Edit (or a double click) opens it for writing;
 * the kind chip changes its kind. */
import { createEffect, createMemo, For, on, Show } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconEdit, IconTrash } from '../../components/icons';
import CommentEditor, { KindSelect, type EditorResult } from './CommentEditor';
import { renderBody } from './bodyHtml';
import { labelParts } from './labels';
import { mathRenderer } from './mathHtml';
import { shortQuote, type Entry } from './model';
import { citeIndex, flashed } from './markupStore';
import { KIND_LABEL, type CommentKind } from './types';

interface Props {
  entry: Entry;
  editing: boolean;
  /** The comment being written, not yet kept. */
  draft?: boolean;
  onOpen: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onKind: (kind: CommentKind) => void;
  onSave: (result: EditorResult) => void;
  onCancel: () => void;
}

export function LabelLine(props: { parts: string[] }) {
  return (
    <span class="mk-label-line">
      <For each={props.parts}>
        {(part, i) => (
          <>
            <Show when={i() > 0}>
              <span class="mk-sep" aria-hidden="true" />
            </Show>
            <span>{part}</span>
          </>
        )}
      </For>
    </span>
  );
}

export default function CommentCard(props: Props) {
  let card!: HTMLElement;
  const c = () => props.entry.comment;
  const typoFix = () => c().kind === 'typo' && c().replacement !== null;
  const body = createMemo(() =>
    c().body.trim()
      ? renderBody(c().body, {
          math: mathRenderer(),
          cite: (key) => citeIndex().byKey.get(key)?.label ?? null,
        })
      : ''
  );

  // Pointed at from the page: bring it into view and mark it a moment.
  createEffect(
    on(flashed, (f) => {
      if (!f || f.id !== c().id || !card) return;
      card.scrollIntoView?.({ block: 'nearest' });
      card.classList.remove('is-flashed');
      void card.offsetWidth;
      card.classList.add('is-flashed');
    })
  );

  // A new comment's card is brought into view as it opens.
  createEffect(() => {
    if (props.draft && card) card.scrollIntoView?.({ block: 'nearest' });
  });

  return (
    <article
      ref={card}
      class={`mk-card is-${c().kind}`}
      classList={{ 'is-editing': props.editing, 'is-draft': !!props.draft }}
      data-comment={c().id}
      aria-label={`Comment ${props.entry.number}`}
    >
      <div class="mk-card-head">
        <span class={`mk-num-badge is-${c().kind}`}>{props.entry.number}</span>
        <button
          type="button"
          class="mk-card-where"
          title="Show on the page"
          onClick={() => props.onOpen()}
        >
          <LabelLine parts={labelParts(props.entry.label, false)} />
        </button>
        <Show
          when={!props.editing}
          fallback={<span class="mk-card-state">{props.draft ? 'New' : 'Editing'}</span>}
        >
          <KindSelect value={c().kind} onChange={(k) => props.onKind(k)} />
          <span class="mk-card-actions">
            <IconButton label="Edit" onClick={() => props.onEdit()}>
              <IconEdit />
            </IconButton>
            <IconButton label="Delete" onClick={() => props.onDelete()}>
              <IconTrash />
            </IconButton>
          </span>
        </Show>
      </div>
      <Show
        when={props.editing}
        fallback={
          <div
            class="mk-card-body"
            onClick={() => props.onOpen()}
            onDblClick={() => props.onEdit()}
          >
            <Show when={typoFix()}>
              <div class="mk-fix">
                <span class="mk-del">{shortQuote(c().anchor.quote, 80)}</span>
                <Show when={c().replacement} fallback={<span class="mk-dim"> (delete)</span>}>
                  {' '}
                  <span class="mk-ins">{c().replacement}</span>
                </Show>
              </div>
            </Show>
            <Show when={!typoFix() && c().anchor.quote.trim()}>
              <div class="mk-card-quote">{shortQuote(c().anchor.quote, 160)}</div>
            </Show>
            <Show when={!typoFix() && c().anchor.rect}>
              <div class="mk-card-quote is-region">Region on the page</div>
            </Show>
            <Show when={body()}>
              {(html) => (
                // Escaped text and KaTeX output (bodyHtml.ts).
                // eslint-disable-next-line solid/no-innerhtml
                <div class="mk-rendered" innerHTML={html()} />
              )}
            </Show>
            <Show when={!body() && !typoFix()}>
              <div class="mk-dim">{KIND_LABEL[c().kind]}</div>
            </Show>
          </div>
        }
      >
        <Show when={!typoFix() && c().anchor.quote.trim()}>
          <div class="mk-card-quote">{shortQuote(c().anchor.quote, 160)}</div>
        </Show>
        <CommentEditor
          mode={c().kind === 'typo' && c().replacement !== null ? 'typo' : 'comment'}
          quote={c().anchor.quote}
          initial={{
            body: c().body,
            kind: c().kind,
            quickmark: c().quickmark,
            replacement: c().replacement,
          }}
          onSave={(r) => props.onSave(r)}
          onCancel={() => props.onCancel()}
        />
      </Show>
    </article>
  );
}
