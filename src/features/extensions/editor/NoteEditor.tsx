/** The Markdown editor SpeedyF draws for an extension (`readerPanes`,
 * `projectViews`): Obsidian-style live rendering, keyboard first.
 *
 * The note is drawn block by block (`markdown.ts`). The block holding the
 * cursor is a native text field showing its Markdown source - so IME,
 * spell check, plain-text paste and caret movement are the platform's own -
 * and every other block is rendered. Arrows at a block's edge move to the
 * next one, Enter splits a line, Backspace at its start joins it to the one
 * before. Source mode shows the whole note as one text field.
 *
 * Every change goes through the window's session for the note
 * (`noteSession.ts`), which owns undo and talks to the core; the editor
 * holds only where the cursor is. Blocks are kept between edits and only the
 * ones whose text changed are drawn again, and typing within a block does
 * not re-cut the note at all, which is what keeps a long note quick. */
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Match,
  on,
  onCleanup,
  onMount,
  Show,
  Switch,
  untrack,
  type JSX,
} from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import MathText, { MathFormula } from '../../../components/MathText';
import { followExternalLink } from '../../../lib/links/followExternalLink';
import { engine } from '../../../lib/transport/engine';
import {
  citationIndex,
  noteSessions,
  registerEditor,
  type CitationEntry,
  type CitationIndex,
} from '../editorHost';
import { runEditorEvent, type ExtensionEditor } from '../extensionStore';
import { applyOp, diffOp, mapPos, type Op } from './ops';
import {
  codeSource,
  inlineSpans,
  lineShape,
  mathSource,
  sourceOffset,
  type Citation,
  type Span,
} from './markdown';
import type { NoteKey, NoteSession } from './noteSession';
import {
  citeQuery,
  continueList,
  pickerMatches,
  reuseBlocks,
  startsOf,
  type Block,
} from './blocks';

export interface NoteEditorProps {
  extensionId: string;
  projectId: string;
  editor: ExtensionEditor;
  /** Words for an empty note. */
  placeholder?: string;
  /** Extra controls for the header row (the drawer puts its title there). */
  header?: JSX.Element;
  class?: string;
}

export default function NoteEditor(props: NoteEditorProps) {
  const key = (): NoteKey => ({
    extensionId: props.extensionId,
    projectId: props.projectId,
    file: props.editor.file,
  });
  const keyId = () => `${props.extensionId}/${props.projectId}/${props.editor.file}`;
  const [session, setSession] = createSignal<NoteSession | null>(null);

  createEffect(
    on(keyId, () => {
      const all = noteSessions();
      const joined = all.acquire(key());
      setSession(joined);
      onCleanup(() => all.release(joined));
    })
  );

  // Ask the extension what each cite key reads as, now and whenever the
  // library changes.
  const askCitations = () =>
    void runEditorEvent(props.extensionId, props.editor, {
      event: 'citations',
      projectId: props.projectId,
      file: props.editor.file,
    });
  createEffect(on(keyId, askCitations));
  onMount(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void engine
      .onLibraryChanged(() => {
        clearTimeout(timer);
        timer = setTimeout(askCitations, 400);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    onCleanup(() => {
      disposed = true;
      clearTimeout(timer);
      unlisten?.();
    });
  });

  const index = createMemo(() => citationIndex(props.extensionId, props.projectId));

  return (
    <div class={`note-editor ${props.class ?? ''}`}>
      <Show when={session()} keyed>
        {(s) => (
          <Switch>
            <Match when={s.status() === 'loading'}>
              <div class="note-editor-empty">Opening...</div>
            </Match>
            <Match when={s.status() === 'closed'}>
              <div class="note-editor-empty" role="status">
                {s.reason() === 'deleted'
                  ? 'This project was deleted; its notepad was kept.'
                  : 'This note was closed.'}
              </div>
            </Match>
            <Match when={s.status() === 'error'}>
              <div class="note-editor-empty" role="alert">
                Could not open this note: {s.reason()}
              </div>
            </Match>
            <Match when={s.status() === 'ready'}>
              <EditorBody
                session={s}
                index={index()}
                noteKey={key()}
                editor={props.editor}
                projectId={props.projectId}
                extensionId={props.extensionId}
                placeholder={props.placeholder}
                header={props.header}
              />
            </Match>
          </Switch>
        )}
      </Show>
    </div>
  );
}

interface BodyProps {
  session: NoteSession;
  index: CitationIndex;
  noteKey: NoteKey;
  editor: ExtensionEditor;
  projectId: string;
  extensionId: string;
  placeholder: string | undefined;
  header: JSX.Element | undefined;
}

/** Images from the note's own folder, as blob URLs, per note. */
const assetUrls = new Map<string, Promise<string | null>>();

function assetUrl(noteKey: NoteKey, path: string): Promise<string | null> {
  const id = `${noteKey.extensionId}/${noteKey.projectId}/${path}`;
  let hit = assetUrls.get(id);
  if (!hit) {
    hit = invoke<{ base64: string | null } | null>('project_file_read', {
      extensionId: noteKey.extensionId,
      projectId: noteKey.projectId,
      path,
      kind: 'bytes',
    })
      .then((body) => {
        if (!body?.base64) return null;
        const binary = atob(body.base64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        const type = path.endsWith('.svg')
          ? 'image/svg+xml'
          : /\.jpe?g$/i.test(path)
            ? 'image/jpeg'
            : 'image/png';
        return URL.createObjectURL(new Blob([bytes], { type }));
      })
      .catch(() => null);
    if (assetUrls.size > 200) assetUrls.clear();
    assetUrls.set(id, hit);
  }
  return hit;
}

function EditorBody(props: BodyProps) {
  const s = untrack(() => props.session);
  const [blocks, setBlocks] = createSignal<Block[]>(reuseBlocks([], s.text()));
  const [active, setActive] = createSignal<Block | null>(null);
  const [source, setSource] = createSignal(false);
  const [flash, setFlash] = createSignal<number | null>(null);
  const [picker, setPicker] = createSignal<{ at: number; query: string; index: number } | null>(
    null
  );
  /** Absolute caret while a block is being edited. */
  let caret = 0;
  /** Where the active block starts, and its text as last committed. */
  let activeStart = 0;
  let activeRaw = '';
  let composing = false;
  /** An outside change arrived mid-composition, to apply once it ends. */
  let deferredExternal = false;
  let field: HTMLTextAreaElement | undefined;
  let sourceField: HTMLTextAreaElement | undefined;
  let scroller!: HTMLDivElement;

  const rebuild = () => setBlocks((prev) => reuseBlocks(prev, s.text()));

  /** Make the block holding `offset` the edited one, with the caret there. */
  const activateAt = (offset: number, select?: [number, number]) => {
    const list = blocks();
    const starts = startsOf(list);
    let at = list.length - 1;
    for (let i = 0; i < list.length; i++) {
      if (offset <= starts[i]! + list[i]!.raw.length) {
        at = i;
        break;
      }
    }
    const block = list[at];
    if (!block) return;
    activeStart = starts[at]!;
    activeRaw = block.raw;
    caret = Math.min(Math.max(offset, activeStart), activeStart + block.raw.length);
    setPicker(null);
    if (active() === block && field) {
      field.value = block.raw;
      placeCaret(select);
      return;
    }
    setActive(block);
    queueMicrotask(() => placeCaret(select));
  };

  const placeCaret = (select?: [number, number]) => {
    if (!field) return;
    if (field.value !== activeRaw) field.value = activeRaw;
    autosize(field);
    field.focus({ preventScroll: true });
    const local = caret - activeStart;
    if (select) field.setSelectionRange(select[0] - activeStart, select[1] - activeStart);
    else field.setSelectionRange(local, local);
  };

  const deactivate = () => {
    if (!active()) return;
    setActive(null);
    setPicker(null);
    rebuild();
  };

  /** Commit a local edit made in the active field, or by a key. */
  const commit = (op: Op, before: number, after: number, restructure: boolean) => {
    s.edit(op, before, after);
    caret = after;
    if (restructure) {
      rebuild();
      activateAt(after);
    }
  };

  // Changes from elsewhere: another view, another window, a reload.
  createEffect(
    on(
      () => s.external(),
      (change) => {
        if (change.seq === 0) return;
        if (composing) {
          deferredExternal = true;
          return;
        }
        applyExternal(change.op);
      },
      { defer: true }
    )
  );

  const applyExternal = (op: Op | null) => {
    const wasActive = active() !== null;
    if (op && wasActive) caret = mapPos(op, caret);
    rebuild();
    if (source()) {
      if (sourceField) {
        const start = sourceField.selectionStart;
        const end = sourceField.selectionEnd;
        sourceField.value = s.text();
        if (op) sourceField.setSelectionRange(mapPos(op, start), mapPos(op, end));
      }
      return;
    }
    if (wasActive) activateAt(op ? caret : Math.min(caret, s.text().length));
  };

  /* ---- typing in a block ---- */

  const onInput = (e: InputEvent & { currentTarget: HTMLTextAreaElement }) => {
    const ta = e.currentTarget;
    autosize(ta);
    const value = ta.value;
    const local = diffOp(activeRaw, value);
    const before = caret;
    activeRaw = value;
    if (!local) return;
    const op: Op = {
      from: activeStart + local.from,
      to: activeStart + local.to,
      insert: local.insert,
    };
    const after = activeStart + ta.selectionStart;
    const block = active();
    const structural =
      !composing &&
      block?.kind === 'line' &&
      (value.includes('\n') ||
        value.trimStart().startsWith('$$') ||
        /^\s{0,3}(```|~~~)/.test(value));
    commit(op, before, after, structural);
    if (!composing && !structural) updatePicker(value, ta.selectionStart);
  };

  const updatePicker = (value: string, localCaret: number) => {
    const found = citeQuery(value, localCaret);
    if (!found || props.index.entries.every((e) => !e.pick)) {
      setPicker(null);
      return;
    }
    const prev = picker();
    setPicker({ at: activeStart + found.at, query: found.query, index: prev?.index ?? 0 });
  };

  const matches = createMemo(() => {
    const p = picker();
    return p ? pickerMatches(props.index.entries, p.query) : [];
  });

  const pick = (entry: CitationEntry) => {
    const p = picker();
    if (!p || !field) return;
    const insert = `@${entry.key}`;
    const op: Op = { from: p.at, to: caret, insert };
    const after = p.at + insert.length;
    activeRaw = applyOp(activeRaw, { from: p.at - activeStart, to: caret - activeStart, insert });
    field.value = activeRaw;
    commit(op, caret, after, false);
    setPicker(null);
    placeCaret();
  };

  const undoRedo = (redo: boolean) => {
    const at = redo ? s.redo() : s.undo();
    if (at === null) return;
    rebuild();
    if (source()) {
      if (sourceField) {
        sourceField.value = s.text();
        sourceField.setSelectionRange(at, at);
      }
      return;
    }
    activateAt(Math.min(at, s.text().length));
  };

  const onKeyDown = (e: KeyboardEvent & { currentTarget: HTMLTextAreaElement }) => {
    const ta = e.currentTarget;
    const mod = /mac/i.test(navigator.userAgent) ? e.metaKey : e.ctrlKey;
    const p = picker();
    if (p && matches().length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const n = matches().length;
        setPicker({ ...p, index: (p.index + (e.key === 'ArrowDown' ? 1 : n - 1)) % n });
        return;
      }
      if ((e.key === 'Enter' || e.key === 'Tab') && !e.isComposing) {
        e.preventDefault();
        const entry = matches()[Math.min(p.index, matches().length - 1)];
        if (entry) pick(entry);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        setPicker(null);
        return;
      }
    }
    if (e.isComposing || e.keyCode === 229) return;
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      undoRedo(e.shiftKey);
      return;
    }
    if (mod && !e.shiftKey && e.key.toLowerCase() === 'y' && !/mac/i.test(navigator.userAgent)) {
      e.preventDefault();
      undoRedo(true);
      return;
    }
    if (mod && e.key.toLowerCase() === 'a') {
      if (ta.selectionStart === 0 && ta.selectionEnd === ta.value.length) {
        e.preventDefault();
        enterSource(true);
      }
      return;
    }
    const block = active();
    if (!block) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const collapsed = start === end;
    const list = blocks();
    const index = list.indexOf(block);
    if (e.key === 'Enter' && !e.shiftKey && block.kind === 'line') {
      e.preventDefault();
      const tail = ta.value.slice(end);
      const marker = tail === '' ? continueList(ta.value) : null;
      if (marker === '') {
        // An empty list item ends the list: the marker goes.
        const op: Op = { from: activeStart, to: activeStart + ta.value.length, insert: '' };
        commit(op, caret, activeStart, true);
        return;
      }
      const insert = `\n${marker ?? ''}`;
      const op: Op = { from: activeStart + start, to: activeStart + end, insert };
      commit(op, activeStart + start, activeStart + start + insert.length, true);
      return;
    }
    if (e.key === 'Backspace' && collapsed && start === 0 && activeStart > 0) {
      e.preventDefault();
      commit({ from: activeStart - 1, to: activeStart, insert: '' }, caret, activeStart - 1, true);
      return;
    }
    if (e.key === 'Delete' && collapsed && start === ta.value.length && index < list.length - 1) {
      e.preventDefault();
      const at = activeStart + ta.value.length;
      commit({ from: at, to: at + 1, insert: '' }, caret, at, true);
      return;
    }
    const atStart = collapsed && start === 0;
    const atEnd = collapsed && end === ta.value.length;
    if ((e.key === 'ArrowUp' || e.key === 'ArrowLeft') && atStart && index > 0 && !e.shiftKey) {
      e.preventDefault();
      deactivateTo(activeStart - 1);
      return;
    }
    if (
      (e.key === 'ArrowDown' || e.key === 'ArrowRight') &&
      atEnd &&
      index < list.length - 1 &&
      !e.shiftKey
    ) {
      e.preventDefault();
      deactivateTo(activeStart + ta.value.length + 1);
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      deactivate();
      field?.blur();
    }
  };

  /** Move the caret to `offset`, re-cutting the note first so the block it
   * lands in is current. */
  const deactivateTo = (offset: number) => {
    rebuild();
    activateAt(offset);
  };

  /* ---- source mode ---- */

  let sourceValue = '';
  const enterSource = (selectAll = false) => {
    deactivate();
    setSource(true);
    queueMicrotask(() => {
      if (!sourceField) return;
      sourceValue = s.text();
      sourceField.value = sourceValue;
      sourceField.focus();
      if (selectAll) sourceField.select();
    });
  };
  const leaveSource = () => {
    setSource(false);
    rebuild();
  };
  const onSourceInput = (e: InputEvent & { currentTarget: HTMLTextAreaElement }) => {
    const value = e.currentTarget.value;
    const op = diffOp(s.text(), value);
    if (!op) return;
    const before = op.from;
    s.edit(op, before, e.currentTarget.selectionStart);
    sourceValue = value;
  };
  const onSourceKey = (e: KeyboardEvent & { currentTarget: HTMLTextAreaElement }) => {
    if (e.isComposing || e.keyCode === 229) return;
    const mod = /mac/i.test(navigator.userAgent) ? e.metaKey : e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      undoRedo(e.shiftKey);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      leaveSource();
    }
  };

  /* ---- clicks on rendered text ---- */

  const activateCitation = (cite: Citation) => {
    void runEditorEvent(props.extensionId, props.editor, {
      event: 'activate',
      projectId: props.projectId,
      file: props.editor.file,
      cite: { key: cite.key, locator: cite.locator, page: cite.page, box: cite.box },
    });
  };

  const onBlockClick = (e: MouseEvent, block: Block) => {
    const target = e.target as HTMLElement;
    if (target.closest('a, .np-cite, button')) return;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed) return;
    const element = e.currentTarget as HTMLElement;
    const list = blocks();
    const start = startsOf(list)[list.indexOf(block)] ?? 0;
    let drawnOffset = element.textContent?.length ?? 0;
    const doc = document as Document & {
      caretRangeFromPoint?: (x: number, y: number) => Range | null;
    };
    const range = doc.caretRangeFromPoint?.(e.clientX, e.clientY);
    if (range && element.contains(range.startContainer)) {
      const pre = document.createRange();
      pre.selectNodeContents(element);
      pre.setEnd(range.startContainer, range.startOffset);
      drawnOffset = pre.toString().length;
    }
    activateAt(start + sourceOffset(block.raw, element.textContent ?? '', drawnOffset));
  };

  /** A selection across rendered blocks copies their Markdown. */
  const onCopy = (e: ClipboardEvent) => {
    if (e.target instanceof HTMLTextAreaElement) return;
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed || !e.clipboardData) return;
    const blockOf = (node: Node | null) =>
      (node instanceof HTMLElement ? node : node?.parentElement)?.closest<HTMLElement>(
        '[data-block]'
      );
    const a = blockOf(selection.anchorNode);
    const b = blockOf(selection.focusNode);
    if (!a || !b || a === b) return;
    const list = blocks();
    const ia = list.findIndex((x) => String(x.id) === a.dataset.block);
    const ib = list.findIndex((x) => String(x.id) === b.dataset.block);
    if (ia < 0 || ib < 0) return;
    const [from, to] = ia < ib ? [ia, ib] : [ib, ia];
    e.preventDefault();
    e.clipboardData.setData(
      'text/plain',
      list
        .slice(from, to + 1)
        .map((x) => x.raw)
        .join('\n')
    );
  };

  /* ---- reveal and insert, for the extension ---- */

  onMount(() => {
    const unregister = registerEditor(props.noteKey, {
      insertAtCaret: (text) => {
        if (!active()) return false;
        // As a paragraph of its own after the block being edited.
        const end = activeStart + activeRaw.length;
        const insert = `\n${text.replace(/\n+$/, '')}`;
        commit({ from: end, to: end, insert }, caret, end + insert.length, true);
        return true;
      },
      reveal: (offset) => {
        if (source()) leaveSource();
        deactivate();
        const list = blocks();
        const starts = startsOf(list);
        let at = list.length - 1;
        for (let i = 0; i < list.length; i++) {
          if (offset <= starts[i]! + list[i]!.raw.length) {
            at = i;
            break;
          }
        }
        const block = list[at];
        if (!block) return;
        setFlash(block.id);
        setTimeout(() => setFlash((id) => (id === block.id ? null : id)), 1600);
        queueMicrotask(() =>
          scroller?.querySelector(`[data-block="${block.id}"]`)?.scrollIntoView({ block: 'center' })
        );
      },
    });
    onCleanup(unregister);
  });

  const empty = () => blocks().length === 1 && blocks()[0]!.raw === '' && !active();

  return (
    <>
      <div class="note-editor-head">
        {props.header}
        <span class="note-editor-spacer" />
        <Show when={s.notice()}>
          {(notice) => (
            <span class="note-editor-notice" role="status" title={notice()}>
              {notice()}
              <button type="button" class="paper-mini-btn" onClick={() => s.dismissNotice()}>
                OK
              </button>
            </span>
          )}
        </Show>
        <button
          type="button"
          class="paper-mini-btn"
          aria-pressed={source()}
          title="Edit the whole note as plain Markdown (Escape returns)"
          onClick={() => (source() ? leaveSource() : enterSource())}
        >
          {source() ? 'Done' : 'Source'}
        </button>
      </div>
      <div class="note-editor-scroll" ref={scroller} onCopy={onCopy}>
        <Show
          when={!source()}
          fallback={
            <textarea
              ref={sourceField}
              class="note-source"
              spellcheck={true}
              aria-label="Note source"
              onInput={onSourceInput}
              onKeyDown={onSourceKey}
              onCompositionStart={() => (composing = true)}
              onCompositionEnd={() => (composing = false)}
            />
          }
        >
          <div class="note-blocks" role="document">
            <Show when={empty()}>
              <div class="note-placeholder" onClick={() => activateAt(0)}>
                {props.placeholder ?? 'Empty. Click to write.'}
              </div>
            </Show>
            <For each={blocks()}>
              {(block) => (
                <Show
                  when={active() === block}
                  fallback={
                    <Show when={!empty()}>
                      <div
                        class="note-block"
                        classList={{ 'is-flash': flash() === block.id }}
                        data-block={block.id}
                        onClick={(e) => onBlockClick(e, block)}
                      >
                        {drawBlock({
                          block,
                          index: () => props.index,
                          noteKey: props.noteKey,
                          onCite: activateCitation,
                        })}
                      </div>
                    </Show>
                  }
                >
                  <div class="note-block is-active" data-block={block.id}>
                    <textarea
                      ref={(el) => {
                        field = el;
                        el.value = activeRaw;
                      }}
                      class="note-field"
                      classList={{ 'is-mono': block.kind !== 'line' }}
                      rows={1}
                      spellcheck={true}
                      aria-label="Note line"
                      onInput={onInput}
                      onKeyDown={onKeyDown}
                      onCompositionStart={() => (composing = true)}
                      onCompositionEnd={() => {
                        composing = false;
                        if (deferredExternal) {
                          deferredExternal = false;
                          applyExternal(null);
                        }
                      }}
                      onBlur={(e) => {
                        // Focus moving to the picker or the head is not leaving.
                        const next = e.relatedTarget as HTMLElement | null;
                        if (next?.closest('.note-picker')) return;
                        if (!composing) deactivate();
                      }}
                    />
                    <Show when={picker() && matches().length > 0}>
                      <div class="note-picker" role="listbox" aria-label="Cite an item">
                        <For each={matches()}>
                          {(entry, i) => (
                            <div
                              role="option"
                              class="note-picker-row"
                              aria-selected={i() === (picker()?.index ?? 0)}
                              classList={{ 'is-current': i() === (picker()?.index ?? 0) }}
                              onMouseDown={(e) => {
                                e.preventDefault();
                                pick(entry);
                              }}
                            >
                              <span class="note-picker-title">
                                <MathText text={entry.title || entry.label} />
                              </span>
                              <span class="note-picker-detail">{entry.detail}</span>
                            </div>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                </Show>
              )}
            </For>
          </div>
        </Show>
      </div>
    </>
  );
}

function autosize(el: HTMLTextAreaElement): void {
  el.style.height = 'auto';
  el.style.height = `${el.scrollHeight}px`;
}

/* ---- drawing ---- */

/** Formulas are typeset when they come into view, not before: a long note
 * with a thousand of them opens as fast as one with none. */
let observer: IntersectionObserver | null = null;
const onVisible = new WeakMap<Element, () => void>();

function whenVisible(el: Element, show: () => void): () => void {
  if (typeof IntersectionObserver === 'undefined') {
    show();
    return () => {};
  }
  observer ??= new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        onVisible.get(entry.target)?.();
        onVisible.delete(entry.target);
        observer?.unobserve(entry.target);
      }
    },
    { rootMargin: '400px 0px' }
  );
  onVisible.set(el, show);
  observer.observe(el);
  return () => {
    onVisible.delete(el);
    observer?.unobserve(el);
  };
}

function LazyMath(props: { tex: string; display: boolean }) {
  const [shown, setShown] = createSignal(false);
  let el!: HTMLSpanElement;
  onMount(() => onCleanup(whenVisible(el, () => setShown(true))));
  return (
    <span ref={el} class={props.display ? 'np-math-block' : 'np-math'}>
      <Show
        when={shown()}
        fallback={<code class="math-raw">{props.display ? props.tex : `$${props.tex}$`}</code>}
      >
        <Show when={props.display} fallback={<MathText text={`$${props.tex}$`} />}>
          <MathFormula tex={props.tex} display={true} />
        </Show>
      </Show>
    </span>
  );
}

function NoteImage(props: { src: string; alt: string; noteKey: NoteKey }) {
  const [url, setUrl] = createSignal<string | null>(null);
  // Only the note's own files: an image on the web would be a request the
  // note made without asking.
  const local = () =>
    /^[A-Za-z0-9_-][A-Za-z0-9._/-]*$/.test(props.src) && !props.src.includes('..');
  createEffect(() => {
    if (!local()) return;
    void assetUrl(props.noteKey, props.src).then(setUrl);
  });
  return (
    <Show when={url()} fallback={<span class="np-image-missing">{props.alt || props.src}</span>}>
      {(u) => <img class="np-image" src={u()} alt={props.alt} />}
    </Show>
  );
}

function CiteToken(props: {
  cite: Citation;
  raw: string;
  index: CitationIndex;
  onCite: (c: Citation) => void;
}) {
  const entry = () => props.index.byKey.get(props.cite.key);
  const text = () => {
    const e = entry();
    if (!e) return props.raw;
    return props.cite.locator ? `${e.label}, ${props.cite.locator}` : e.label;
  };
  return (
    <button
      type="button"
      class="np-cite"
      classList={{ 'is-unresolved': !entry() }}
      title={
        entry()
          ? `${entry()!.title}${props.cite.page !== null ? ` - p. ${props.cite.page + 1}` : ''}`
          : `No item has the key ${props.cite.key}`
      }
      onClick={() => props.onCite(props.cite)}
    >
      {text()}
    </button>
  );
}

function Spans(props: {
  spans: Span[];
  index: CitationIndex;
  noteKey: NoteKey;
  onCite: (c: Citation) => void;
}) {
  return (
    <For each={props.spans}>
      {(span) => {
        switch (span.kind) {
          case 'text':
            return span.text;
          case 'em':
            return (
              <em>
                <Spans {...props} spans={span.children} />
              </em>
            );
          case 'strong':
            return (
              <strong>
                <Spans {...props} spans={span.children} />
              </strong>
            );
          case 'code':
            return <code class="np-code">{span.text}</code>;
          case 'math':
            return <LazyMath tex={span.tex} display={false} />;
          case 'link':
            return (
              <a
                class="np-link"
                href={span.href}
                onClick={(e) => {
                  e.preventDefault();
                  void followExternalLink(span.href);
                }}
              >
                <Spans {...props} spans={span.children} />
              </a>
            );
          case 'image':
            return <NoteImage src={span.src} alt={span.alt} noteKey={props.noteKey} />;
          case 'cite':
            return (
              <CiteToken
                cite={span.cite}
                raw={span.raw}
                index={props.index}
                onCite={props.onCite}
              />
            );
        }
      }}
    </For>
  );
}

/** A block as drawn. A plain function rather than a component: a block
 * never changes (an edited one is a new block), so it is drawn once. */
function drawBlock(args: {
  block: Block;
  index: () => CitationIndex;
  noteKey: NoteKey;
  onCite: (c: Citation) => void;
}): JSX.Element {
  const block = args.block;
  if (block.kind === 'math') {
    return <LazyMath tex={mathSource(block.raw)} display={true} />;
  }
  if (block.kind === 'code') {
    return <pre class="np-pre">{codeSource(block.raw)}</pre>;
  }
  const shape = lineShape(block.raw);
  const inline = (text: string) => (
    <Spans
      spans={inlineSpans(text)}
      index={args.index()}
      noteKey={args.noteKey}
      onCite={args.onCite}
    />
  );
  switch (shape.kind) {
    case 'blank':
      return <div class="np-blank" />;
    case 'rule':
      return <hr class="np-rule" />;
    case 'heading':
      return <div class={`np-h np-h${shape.level}`}>{inline(shape.body)}</div>;
    case 'bullet':
      return (
        <div class="np-li" style={{ 'padding-left': `${1.2 + shape.indent * 0.5}em` }}>
          <span class="np-bullet" aria-hidden="true" />
          {inline(shape.body)}
        </div>
      );
    case 'ordered':
      return (
        <div class="np-li" style={{ 'padding-left': `${1.6 + shape.indent * 0.5}em` }}>
          <span class="np-num">{shape.marker}</span>
          {inline(shape.body)}
        </div>
      );
    case 'quote':
      return (
        <div class="np-quote" style={{ 'margin-left': `${(shape.depth - 1) * 0.8}em` }}>
          {inline(shape.body)}
        </div>
      );
    case 'para':
      return <div class="np-p">{inline(shape.body)}</div>;
  }
}
