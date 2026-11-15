/** The box a comment is written in, in its card in the drawer.
 *
 * Enter keeps it, Shift+Enter starts a new line, Escape lets it go. "/" at
 * the start lists the QuickMarks (one pick writes its words and sets the
 * kind; typing on after it is fine). "@" lists the library: a pick writes
 * `[@citekey]`, read as "[Jar97]". `$...$` is math, typeset live below the
 * box. For a typo fix the box takes the words that should replace the
 * struck ones. */
import { createMemo, createSignal, For, onMount, Show } from 'solid-js';
import { atQuery, insertCitation, pickCandidates, type LibraryEntry } from './citations';
import { renderBody } from './bodyHtml';
import { loadMath, mathRenderer } from './mathHtml';
import { citeIndex } from './markupStore';
import {
  applyQuickmark,
  matchQuickmarks,
  quickmarkById,
  slashQuery,
  type QuickMark,
} from './quickmarks';
import { COMMENT_KINDS, KIND_LABEL, type CommentKind } from './types';
import { shortQuote } from './model';

export interface EditorResult {
  body: string;
  kind: CommentKind;
  quickmark: string | null;
  replacement: string | null;
}

interface Props {
  mode: 'comment' | 'typo';
  /** The words a typo fix strikes. */
  quote: string;
  initial: EditorResult;
  onSave: (result: EditorResult) => void;
  onCancel: () => void;
}

type Menu =
  | { kind: 'quick'; rows: QuickMark[]; index: number }
  | { kind: 'cite'; rows: LibraryEntry[]; index: number; at: number };

/** Kinds in a chip's menu. */
export function KindSelect(props: {
  value: CommentKind;
  onChange: (kind: CommentKind) => void;
  label?: string;
}) {
  return (
    <span class={`mk-kind-chip is-${props.value}`}>
      <select
        class="mk-kind-select"
        aria-label={props.label ?? 'Kind of comment'}
        value={props.value}
        onChange={(e) => props.onChange(e.currentTarget.value as CommentKind)}
        onKeyDown={(e) => e.stopPropagation()}
      >
        <For each={COMMENT_KINDS}>{(kind) => <option value={kind}>{KIND_LABEL[kind]}</option>}</For>
      </select>
    </span>
  );
}

export default function CommentEditor(props: Props) {
  const start = props.initial;
  const [body, setBody] = createSignal(start.body);
  const [replacement, setReplacement] = createSignal(start.replacement ?? '');
  const [kind, setKind] = createSignal<CommentKind>(start.kind);
  const [quickmark, setQuickmark] = createSignal<string | null>(start.quickmark);
  const [menu, setMenu] = createSignal<Menu | null>(null);
  let field!: HTMLTextAreaElement;
  let fix: HTMLInputElement | undefined;

  void loadMath();

  const grow = () => {
    field.style.height = 'auto';
    field.style.height = `${field.scrollHeight}px`;
  };

  onMount(() => {
    const target = props.mode === 'typo' && fix ? fix : field;
    target.focus();
    if (target === field) field.setSelectionRange(field.value.length, field.value.length);
    grow();
  });

  const updateMenu = () => {
    const text = field.value;
    const caret = field.selectionStart;
    const slash = slashQuery(text, caret);
    if (slash !== null) {
      const rows = matchQuickmarks(slash);
      setMenu(rows.length > 0 ? { kind: 'quick', rows, index: 0 } : null);
      return;
    }
    const at = atQuery(text, caret);
    if (at) {
      const rows = pickCandidates(citeIndex(), at.query);
      setMenu(rows.length > 0 ? { kind: 'cite', rows, index: 0, at: at.at } : null);
      return;
    }
    setMenu(null);
  };

  const pickQuick = (mark: QuickMark) => {
    const out = applyQuickmark(field.value, mark);
    field.value = out.text;
    setBody(out.text);
    setKind(mark.kind);
    setQuickmark(mark.id);
    setMenu(null);
    field.setSelectionRange(out.caret, out.caret);
    grow();
  };

  const pickCite = (entry: LibraryEntry, at: number) => {
    const out = insertCitation(field.value, at, field.selectionStart, entry.key);
    field.value = out.text;
    setBody(out.text);
    setMenu(null);
    field.setSelectionRange(out.caret, out.caret);
    grow();
  };

  const pickCurrent = () => {
    const m = menu();
    if (!m) return;
    if (m.kind === 'quick') pickQuick(m.rows[m.index]!);
    else pickCite(m.rows[m.index]!, m.at);
  };

  const result = (): EditorResult => {
    const text = body();
    // A QuickMark whose words were taken out again no longer applies.
    const mark = quickmarkById(quickmark());
    const kept = mark && text.toLowerCase().includes(mark.text.toLowerCase()) ? mark.id : null;
    return {
      body: text,
      kind: kind(),
      quickmark: props.mode === 'typo' ? (quickmark() ?? 'typo') : kept,
      replacement: props.mode === 'typo' ? replacement() : null,
    };
  };

  const onKey = (e: KeyboardEvent) => {
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    const m = menu();
    if (m) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const n = m.rows.length;
        setMenu({ ...m, index: (m.index + (e.key === 'ArrowDown' ? 1 : n - 1)) % n } as Menu);
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        pickCurrent();
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setMenu(null);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      props.onSave(result());
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      props.onCancel();
    }
  };

  const preview = createMemo(() => {
    const text = body();
    if (!/\$|\\\(|\\\[|@/.test(text)) return null;
    return renderBody(text, {
      math: mathRenderer(),
      cite: (key) => citeIndex().byKey.get(key)?.label ?? null,
    });
  });

  return (
    <div class="mk-editor" classList={{ 'is-typo': props.mode === 'typo' }}>
      <Show when={props.mode === 'typo'}>
        <div class="mk-fix-row">
          <span class="mk-fix-old" title={props.quote}>
            {shortQuote(props.quote, 60)}
          </span>
          <input
            ref={fix}
            class="mk-fix-input"
            type="text"
            value={replacement()}
            placeholder="Replace with (empty deletes)"
            aria-label="Replace with"
            spellcheck={false}
            onInput={(e) => setReplacement(e.currentTarget.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.isComposing || e.keyCode === 229) return;
              if (e.key === 'Enter') {
                e.preventDefault();
                props.onSave(result());
              } else if (e.key === 'Escape') {
                e.preventDefault();
                props.onCancel();
              }
            }}
          />
        </div>
      </Show>
      <div class="mk-editor-field">
        <textarea
          ref={field}
          class="mk-textarea"
          rows={props.mode === 'typo' ? 1 : 2}
          value={body()}
          placeholder={
            props.mode === 'typo'
              ? 'Note (optional)'
              : 'Comment. Type / for QuickMarks, @ to cite, $x$ for math.'
          }
          aria-label={props.mode === 'typo' ? 'Note' : 'Comment'}
          onInput={(e) => {
            setBody(e.currentTarget.value);
            grow();
            updateMenu();
          }}
          onClick={updateMenu}
          onKeyDown={onKey}
          onBlur={() => setMenu(null)}
        />
        <Show when={menu()}>
          {(m) => (
            <div
              class="mk-menu"
              role="listbox"
              aria-label={m().kind === 'quick' ? 'QuickMarks' : 'Cite an item'}
            >
              <Show
                when={m().kind === 'quick'}
                fallback={
                  <For each={(m() as Extract<Menu, { kind: 'cite' }>).rows}>
                    {(entry, i) => (
                      <div
                        role="option"
                        class="mk-menu-row"
                        aria-selected={i() === m().index}
                        classList={{ 'is-current': i() === m().index }}
                        onMouseDown={(e) => {
                          e.preventDefault();
                          pickCite(entry, (m() as Extract<Menu, { kind: 'cite' }>).at);
                        }}
                      >
                        <span class="mk-menu-title">{entry.title || entry.key}</span>
                        <span class="mk-menu-detail">
                          [{entry.label}] {entry.detail}
                        </span>
                      </div>
                    )}
                  </For>
                }
              >
                <For each={(m() as Extract<Menu, { kind: 'quick' }>).rows}>
                  {(mark, i) => (
                    <div
                      role="option"
                      class="mk-menu-row is-quick"
                      aria-selected={i() === m().index}
                      classList={{ 'is-current': i() === m().index }}
                      onMouseDown={(e) => {
                        e.preventDefault();
                        pickQuick(mark);
                      }}
                    >
                      <span class="mk-menu-title">{mark.text}</span>
                      <span class={`mk-menu-kind is-${mark.kind}`}>/{mark.id}</span>
                    </div>
                  )}
                </For>
              </Show>
            </div>
          )}
        </Show>
      </div>
      <Show when={preview()}>
        {(html) => (
          // Escaped text and KaTeX output from `trust: false` (bodyHtml.ts).
          // eslint-disable-next-line solid/no-innerhtml
          <div class="mk-preview mk-rendered" innerHTML={html()} />
        )}
      </Show>
      <div class="mk-editor-foot">
        <Show when={props.mode === 'comment'}>
          <KindSelect value={kind()} onChange={setKind} />
        </Show>
        <span class="mk-editor-hint">Enter saves, Shift+Enter new line, Esc cancels</span>
        <button
          type="button"
          class="mk-text-btn"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => props.onCancel()}
        >
          Cancel
        </button>
        <button
          type="button"
          class="mk-text-btn is-primary"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => props.onSave(result())}
        >
          Save
        </button>
      </div>
    </div>
  );
}
