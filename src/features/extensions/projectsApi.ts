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
  equationFor,
  insertIntoNote,
  readCitationEntries,
  revealInEditor,
  setCitations,
  setItemLine,
} from './editorHost';
import type { ExtensionEditor, ExtensionRecord } from './extensionStore';
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

/** `reader.context`: the reader's item, its projects, the project the
 * reader is working in, the page, and the selection's place. */
function readerContext(): unknown {
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  const item = readerItem();
  const geometry = selectedGeometry();
  let selection: { page: number; box: [number, number, number, number]; text: string } | null =
    null;
  if (geometry && doc?.loaded && geometry.docId === doc.docId && geometry.rects.length > 0) {
    const left = Math.min(...geometry.rects.map((r) => r[0]));
    const top = Math.min(...geometry.rects.map((r) => r[1]));
    const right = Math.max(...geometry.rects.map((r) => r[0] + r[2]));
    const bottom = Math.max(...geometry.rects.map((r) => r[1] + r[3]));
    const round = (v: number) => Math.round(v);
    selection = {
      page: geometry.page,
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
      const text = record.text;
      if (typeof text !== 'string' || !text.trim()) throw new Error('expected a non-empty "text"');
      if (text.length > 200_000) throw new Error('editor.insert takes at most 200 000 characters');
      const at = record.at === 'cursor' ? 'cursor' : 'end';
      const offset = await insertIntoNote({ extensionId: extension.id, projectId, file }, text, at);
      return { offset };
    }
    case 'editor.reveal': {
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
    ref: ReturnType<typeof equationFor>
  ) => Promise<{ latex: string; display: boolean } | null>
): Promise<unknown> {
  const record = (args ?? {}) as Record<string, unknown>;
  switch (api) {
    case 'equations.convert': {
      const token = record.equation;
      const ref = equationFor(token);
      const answer = await convert(token as string, ref);
      return answer ? { latex: answer.latex, display: answer.display } : null;
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
