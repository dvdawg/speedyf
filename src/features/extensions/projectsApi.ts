/** The project APIs behind the gate: `projects:read`, `projects:files`
 * and the editor calls, plus the calls that need no permission because
 * they only draw into what the extension declared or hand back what the
 * user picked (`OPEN_APIS`). `extensionStore.handleCall` has already
 * checked the permission; every argument is still untrusted. */
import { invoke } from '@tauri-apps/api/core';
import { engine } from '../../lib/transport/engine';
import { selectedGeometry, selectedText } from '../../stores/selectionStore';
import { activeTab } from '../../stores/tabsStore';
import { readBoxArg, readProjectIdArg, readProjectPathArg, toProjectItems } from './api';
import {
  cancelHandle,
  equationFor,
  handlePlace,
  insertIntoNote,
  insertPending,
  markHandle,
  markRange,
  noteShown,
  readCitationEntries,
  replaceInNote,
  addNoteReferences,
  replaceMarked,
  resolvePending,
  revealInEditor,
  setCitations,
  setItemLine,
} from './editorHost';
import type { NoteKey, PendingShape } from './editor/noteSession';
import type { Work } from './editor/references';
import type {
  ConvertMode,
  ConvertedEquation,
  ExtensionEditor,
  ExtensionRecord,
} from './extensionStore';
import { openItemAt } from './navigateItem';
import { askReaderProject, readerItem, readerProjectId } from './readerContext';
import { openProjectView, openReaderPane } from './surfaces';
import type { ExtensionHost } from './host';

/** Largest file an extension may write in one call. */
const MAX_WRITE = 8 * 1024 * 1024;

function bytesToBase64(bytes: number[]): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.slice(i, i + 0x8000));
  }
  return btoa(binary);
}

function base64ToBytes(text: string): number[] {
  const binary = atob(text);
  const out = new Array<number>(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

/** The editor files an extension declared; editor calls name one of them
 * (or mean the first). */
function editorFile(extension: ExtensionRecord, args: unknown): string {
  const files = [
    ...extension.readerPanes.map((p) => p.editor.file),
    ...extension.projectViews.map((v) => v.editor.file),
  ];
  const wanted = (args as { file?: unknown } | undefined)?.file;
  if (wanted === undefined) {
    if (files.length === 0) throw new Error('this extension declares no editor');
    return files[0]!;
  }
  if (typeof wanted !== 'string' || !files.includes(wanted)) {
    throw new Error('"file" must be a file one of this extension\'s editors is bound to');
  }
  return wanted;
}

/** `editor.insert`'s `pending`: what is on its way. */
function readPending(raw: unknown): PendingShape {
  if (typeof raw !== 'object' || raw === null)
    throw new Error('expected "pending" to be an object');
  const r = raw as Record<string, unknown>;
  if (r.kind !== 'equation' && r.kind !== 'image' && r.kind !== 'text') {
    throw new Error('"pending.kind" must be "equation", "image" or "text"');
  }
  let shape: PendingShape['shape'] = null;
  if (r.shape !== undefined && r.shape !== null) {
    const s = r.shape as { w?: unknown; h?: unknown };
    const ok = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v) && v > 0;
    if (typeof s !== 'object' || !ok(s.w) || !ok(s.h)) {
      throw new Error('"pending.shape" must be { w, h }, both positive');
    }
    // Proportions only, kept sane: a sliver is drawn as a thin band.
    shape = { w: s.w, h: Math.min(Math.max(s.h, s.w / 20), s.w * 4) };
  }
  if (r.caption !== undefined && r.caption !== null && typeof r.caption !== 'string') {
    throw new Error('expected "pending.caption" to be a string');
  }
  const caption =
    typeof r.caption === 'string' && r.caption.trim() ? r.caption.slice(0, 2000) : null;
  return { kind: r.kind, shape, caption };
}

/** A status line's words. */
function readLabel(raw: unknown): string {
  if (typeof raw !== 'string' || !raw.trim()) throw new Error('expected a non-empty "label"');
  return raw.trim().slice(0, 120);
}

/** Text for the note, checked for size. */
function readNoteText(raw: unknown, api: string): string {
  if (typeof raw !== 'string' || !raw.trim()) throw new Error('expected a non-empty "text"');
  if (raw.length > 200_000) throw new Error(`${api} takes at most 200 000 characters`);
  return raw;
}

/** A send is about to land in `key`'s note: when no view of it is on
 * screen in this window, open the reader's drawer on it (when the paper on
 * screen is in that project), so the user sees it arrive. */
function showNote(extension: ExtensionRecord, key: NoteKey): void {
  if (noteShown(key)) return;
  const pane = extension.readerPanes.find((p) => p.editor.file === key.file);
  if (!pane || !readerItem()?.projectIds.includes(key.projectId)) return;
  openReaderPane(pane.id, key.projectId);
}

/** `reader.context`: the reader's item, its projects, the project the
 * reader is working in, the page, and the selection's place. */
function readerContext(): unknown {
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  const item = readerItem();
  const geometry = selectedGeometry();
  let selection: { page: number; box: [number, number, number, number]; text: string } | null =
    null;
  // A selection over a page break is linked to where it begins.
  const place = geometry?.start ?? geometry;
  if (geometry && place && doc?.loaded && geometry.docId === doc.docId && place.rects.length > 0) {
    const left = Math.min(...place.rects.map((r) => r[0]));
    const top = Math.min(...place.rects.map((r) => r[1]));
    const right = Math.max(...place.rects.map((r) => r[0] + r[2]));
    const bottom = Math.max(...place.rects.map((r) => r[1] + r[3]));
    const round = (v: number) => Math.round(v);
    selection = {
      page: place.page,
      box: [round(left), round(top), round(right - left), round(bottom - top)],
      text: selectedText(),
    };
  }
  return {
    item: item ? { id: item.id, citeKey: item.citeKey, title: item.title } : null,
    projectIds: item ? [...item.projectIds] : [],
    projectId: readerProjectId(),
    page: doc?.loaded ? (tab?.viewport.state.currentPage ?? null) : null,
    selection,
  };
}

export async function projectsApi(
  extension: ExtensionRecord,
  api: string,
  args: unknown,
  host: () => ExtensionHost
): Promise<unknown> {
  const record = (args ?? {}) as Record<string, unknown>;
  switch (api) {
    case 'projects.list':
      return (await engine.libraryProjects()).map((p) => ({
        id: p.id,
        name: p.name,
        itemCount: p.paperCount,
      }));
    case 'projects.items': {
      const projectId = record.projectId === undefined ? null : readProjectIdArg(args);
      return toProjectItems(await engine.libraryPapers(projectId ? { project: projectId } : {}));
    }
    case 'reader.context':
      return readerContext();
    case 'projects.references': {
      const itemId = record.itemId;
      if (typeof itemId !== 'string' || !itemId) throw new Error('expected an "itemId"');
      return invoke('library_paper_references', { paperId: itemId });
    }
    case 'reader.chooseProject': {
      const resume = host().pauseDeadlines();
      try {
        return await askReaderProject();
      } finally {
        resume();
      }
    }
    case 'navigate.openItem': {
      const itemId = record.itemId;
      if (typeof itemId !== 'string' || !itemId) throw new Error('expected an "itemId"');
      const page = record.page;
      if (
        page !== undefined &&
        page !== null &&
        (typeof page !== 'number' || !Number.isInteger(page) || page < 0)
      ) {
        throw new Error('expected "page" to be a zero-based page number');
      }
      const projectId = record.projectId === undefined ? null : readProjectIdArg(args);
      const opened = await openItemAt(itemId, {
        projectId,
        page: (page as number | undefined) ?? null,
        box: readBoxArg(record.box),
      });
      return { opened };
    }
    case 'projectFiles.read': {
      const kind = record.as === 'bytes' ? 'bytes' : 'text';
      const body = await invoke<{ text: string | null; base64: string | null } | null>(
        'project_file_read',
        {
          extensionId: extension.id,
          projectId: readProjectIdArg(args),
          path: readProjectPathArg(args),
          kind,
        }
      );
      if (!body) return null;
      return kind === 'text' ? (body.text ?? '') : base64ToBytes(body.base64 ?? '');
    }
    case 'projectFiles.write': {
      const projectId = readProjectIdArg(args);
      const path = readProjectPathArg(args);
      const { text, bytes } = record as { text?: unknown; bytes?: unknown };
      if (typeof text === 'string') {
        if (text.length > MAX_WRITE) throw new Error('a project file is limited to 8 MB');
        return {
          written: await invoke<number>('project_file_write', {
            extensionId: extension.id,
            projectId,
            path,
            text,
          }),
        };
      }
      if (
        Array.isArray(bytes) &&
        bytes.length <= MAX_WRITE &&
        bytes.every((b) => typeof b === 'number' && Number.isInteger(b) && b >= 0 && b < 256)
      ) {
        return {
          written: await invoke<number>('project_file_write', {
            extensionId: extension.id,
            projectId,
            path,
            base64: bytesToBase64(bytes as number[]),
          }),
        };
      }
      throw new Error('projectFiles.write needs a string "text" or a "bytes" array of 0-255');
    }
    case 'projectFiles.list':
      return invoke('project_file_list', {
        extensionId: extension.id,
        projectId: readProjectIdArg(args),
      });
    case 'projectFiles.remove':
      return {
        removed: await invoke<boolean>('project_file_remove', {
          extensionId: extension.id,
          projectId: readProjectIdArg(args),
          path: readProjectPathArg(args),
        }),
      };
    case 'editor.citations': {
      const projectId = readProjectIdArg(args);
      const entries = readCitationEntries(record.entries);
      setCitations(extension.id, projectId, entries);
      return { stored: entries.length };
    }
    case 'editor.insert': {
      const projectId = readProjectIdArg(args);
      const file = editorFile(extension, args);
      const key = { extensionId: extension.id, projectId, file };
      const at = record.at === 'cursor' ? 'cursor' : 'end';
      if (record.pending !== undefined) {
        const pending = readPending(record.pending);
        showNote(extension, key);
        return insertPending(key, pending, at, extension.id);
      }
      const text = readNoteText(record.text, 'editor.insert');
      showNote(extension, key);
      return insertIntoNote(key, text, at, extension.id);
    }
    case 'editor.resolve': {
      const text = readNoteText(record.text, 'editor.resolve');
      const done = await resolvePending(extension.id, record.handle, text);
      return done ? { resolved: true, offset: done.offset } : { resolved: false, offset: -1 };
    }
    case 'editor.cancel':
      return { cancelled: cancelHandle(extension.id, record.handle) };
    case 'editor.mark': {
      const label = readLabel(record.label);
      if (record.handle !== undefined) {
        return {
          marked: markHandle(extension.id, record.handle, label),
          handle: record.handle,
        };
      }
      const projectId = readProjectIdArg(args);
      const file = editorFile(extension, args);
      const range = record.range as { from?: unknown; to?: unknown } | undefined;
      const int = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v);
      if (!range || !int(range.from) || !int(range.to)) {
        throw new Error('editor.mark needs a "handle", or a "range" { from, to }');
      }
      const handle = await markRange(
        { extensionId: extension.id, projectId, file },
        extension.id,
        range.from,
        range.to,
        label
      );
      return { marked: true, handle };
    }
    case 'editor.unmark':
      return { cleared: cancelHandle(extension.id, record.handle) };
    case 'editor.replaceMarked': {
      const text = record.text;
      if (typeof text !== 'string') throw new Error('expected a string "text"');
      if (text.length > 200_000) {
        throw new Error('editor.replaceMarked takes at most 200 000 characters');
      }
      return { replaced: await replaceMarked(extension.id, record.handle, text) };
    }
    case 'editor.replace': {
      const projectId = readProjectIdArg(args);
      const file = editorFile(extension, args);
      const { find, replace } = record;
      if (typeof find !== 'string' || !find) throw new Error('expected a non-empty "find"');
      if (typeof replace !== 'string') throw new Error('expected a string "replace"');
      if (find.length > 200_000 || replace.length > 200_000) {
        throw new Error('editor.replace takes at most 200 000 characters');
      }
      const replaced = await replaceInNote(
        { extensionId: extension.id, projectId, file },
        find,
        replace
      );
      return { replaced };
    }
    case 'editor.references': {
      const projectId = readProjectIdArg(args);
      const file = editorFile(extension, args);
      const works = readWorks(record.add);
      const ids = await addNoteReferences({ extensionId: extension.id, projectId, file }, works);
      return { ids };
    }
    case 'editor.reveal': {
      if (record.handle !== undefined) {
        const place = handlePlace(extension.id, record.handle);
        if (!place) return { revealed: false };
        revealInEditor(place.key, place.offset, record.handle as string);
        return { revealed: true };
      }
      const projectId = readProjectIdArg(args);
      const file = editorFile(extension, args);
      const offset = record.offset;
      if (typeof offset !== 'number' || !Number.isInteger(offset) || offset < 0) {
        throw new Error('expected a non-negative integer "offset"');
      }
      revealInEditor({ extensionId: extension.id, projectId, file }, offset);
      return { revealed: true };
    }
  }
  throw new Error(`"${api}" is not an API this version of SpeedyF offers`);
}

/** Calls that need no permission. */
export async function openApi(
  extension: ExtensionRecord,
  api: string,
  args: unknown,
  convert: (
    token: string,
    ref: ReturnType<typeof equationFor>,
    mode?: ConvertMode,
    status?: { owner: string; handle: string }
  ) => Promise<ConvertedEquation | null>
): Promise<unknown> {
  const record = (args ?? {}) as Record<string, unknown>;
  switch (api) {
    case 'equations.convert': {
      const token = record.equation;
      const ref = equationFor(token);
      const mode = record.mode;
      if (mode !== undefined && mode !== 'local' && mode !== 'exact') {
        throw new Error('equations.convert: "mode" must be "local" or "exact"');
      }
      // A status line the converter may name its source in, while it looks.
      const status =
        typeof record.status === 'string'
          ? { owner: extension.id, handle: record.status }
          : undefined;
      const answer = await convert(token as string, ref, mode, status);
      return answer
        ? {
            latex: answer.latex,
            display: answer.display,
            source: answer.source,
            confidence: answer.confidence,
          }
        : null;
    }
    case 'ui.itemLine': {
      const itemId = record.itemId;
      if (typeof itemId !== 'string' || !itemId) throw new Error('expected an "itemId"');
      if (!extension.itemDetailLines) {
        throw new Error('this extension does not contribute itemDetailLines');
      }
      const text = record.text;
      if (text === null || text === undefined || text === '') {
        setItemLine(itemId, null, extension.id);
        return { shown: false };
      }
      if (typeof text !== 'string') throw new Error('expected "text" to be a string or null');
      const action = record.action as { command?: unknown; value?: unknown } | undefined;
      let command: string | null = null;
      if (action !== undefined && action !== null) {
        const declared = (
          (extension.manifest as { contributes?: { commands?: { id: string }[] } } | null)
            ?.contributes?.commands ?? []
        ).map((c) => c.id);
        if (typeof action.command !== 'string' || !declared.includes(action.command)) {
          throw new Error("an item line action must name one of this extension's commands");
        }
        command = action.command;
      }
      setItemLine(
        itemId,
        { extensionId: extension.id, text: text.slice(0, 200), command, value: action?.value },
        extension.id
      );
      return { shown: true };
    }
    case 'pane.open': {
      const pane = extension.readerPanes.find((p) => p.localId === record.pane);
      if (!pane)
        throw new Error(
          `this extension does not contribute a reader pane "${String(record.pane)}"`
        );
      const projectId = record.projectId === undefined ? null : readProjectIdArg(args);
      return { opened: openReaderPane(pane.id, projectId) };
    }
    case 'projectView.open': {
      const view = extension.projectViews.find((v) => v.localId === record.view);
      if (!view)
        throw new Error(
          `this extension does not contribute a project view "${String(record.view)}"`
        );
      const projectId = record.projectId === undefined ? null : readProjectIdArg(args);
      const offset = record.offset;
      if (
        offset !== undefined &&
        (typeof offset !== 'number' || !Number.isInteger(offset) || offset < 0)
      ) {
        throw new Error('expected a non-negative integer "offset"');
      }
      return { opened: openProjectView(view.id, projectId, offset as number | undefined) };
    }
  }
  throw new Error(`"${api}" is not an API this version of SpeedyF offers`);
}

export type { ExtensionEditor };

/** `editor.references` works: `[{ text, ids?, title?, author?, year? }]`. */
function readWorks(raw: unknown): Work[] {
  if (!Array.isArray(raw)) throw new Error('editor.references needs an "add" array');
  if (raw.length > 500) throw new Error('editor.references takes at most 500 works at a time');
  return raw.map((value, i) => {
    const r = (value ?? {}) as Record<string, unknown>;
    if (typeof r.text !== 'string' || !r.text.trim() || r.text.length > 4000) {
      throw new Error(`editor.references: work ${i} needs a "text" of at most 4000 characters`);
    }
    const str = (v: unknown) => (typeof v === 'string' && v.length <= 1000 ? v : null);
    return {
      text: r.text,
      ids: Array.isArray(r.ids)
        ? r.ids.filter((id): id is string => typeof id === 'string' && id.length <= 300)
        : [],
      title: str(r.title),
      author: str(r.author),
      year: typeof r.year === 'number' && Number.isInteger(r.year) ? r.year : null,
    };
  });
}
