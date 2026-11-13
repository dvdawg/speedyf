/** Merging records the engine (or the user) takes for one work: a preprint
 * and its journal version that the scan filed as two items.
 *
 * Always two steps and always inline, never a browser dialog: the first
 * press asks, this says exactly what would move into which record, and the
 * second press merges. The record kept is the one the user was looking at
 * (or the first of a multi-selection); "Keep the other one instead" turns
 * that around without leaving the confirmation. The engine moves files,
 * memberships and identifiers and keeps the best metadata; the files on
 * disk stay where they are, and a merged record can be split again.
 *
 * Also the duplicate line and the merged line of the detail column, which
 * lead here. */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import type { Accessor } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { Paper } from '../../types/engine';
import { projectsStore } from './projectsStore';
import MathText from '../../components/MathText';

/** "Not the same" records the pair with the engine, which then never
 * suggests it again, nor joins the two through a third record. */
export const CAN_DISMISS_DUPLICATES = true;

const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

export function paperTitle(paper: Paper | null | undefined, fallback = 'an item'): string {
  return paper?.meta.title?.value?.trim() || paper?.files[0]?.name || fallback;
}

/** What merging `merged` into `keep` moves: the files, the projects `keep`
 * is not in yet, and the identifiers it lacks. "2 files, 1 project, arXiv
 * id"; empty when nothing would. */
export function mergeMoves(keep: Paper, merged: Paper[]): string[] {
  const files = merged.reduce((sum, p) => sum + p.files.length, 0);
  const projects = new Set(
    merged.flatMap((p) => p.projectIds).filter((id) => !keep.projectIds.includes(id))
  );
  const parts: string[] = [];
  if (files > 0) parts.push(plural(files, 'file'));
  if (projects.size > 0) parts.push(plural(projects.size, 'project'));
  if (!keep.doi && merged.some((p) => p.doi)) parts.push('DOI');
  if (!keep.arxiv && merged.some((p) => p.arxiv)) parts.push('arXiv id');
  if (!keep.isbn && merged.some((p) => p.isbn)) parts.push('ISBN');
  return parts;
}

/** The full records for some ids, read once each and kept for the life of
 * the owner (a merge closes what showed them). Undefined while a read is out; null when it failed. */
export function usePapers(ids: Accessor<string[]>): Accessor<ReadonlyMap<string, Paper | null>> {
  const [papers, setPapers] = createSignal<ReadonlyMap<string, Paper | null>>(new Map());
  const asked = new Set<string>();
  let live = true;
  onCleanup(() => {
    live = false;
  });
  createEffect(() => {
    for (const id of ids()) {
      if (asked.has(id)) continue;
      asked.add(id);
      const keep = (paper: Paper | null) => {
        if (live) setPapers((map) => new Map(map).set(id, paper));
      };
      engine.libraryPaper(id).then(keep, () => keep(null));
    }
  });
  return papers;
}

export function MergeConfirm(props: {
  /** The records to merge; the first is kept unless the user swaps. */
  ids: string[];
  onCancel: () => void;
  onDone?: () => void;
}) {
  const store = projectsStore;
  const papers = usePapers(() => props.ids);
  const [keepIndex, setKeepIndex] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const keepId = () => props.ids[keepIndex() % props.ids.length]!;
  const mergeIds = () => props.ids.filter((id) => id !== keepId());
  const keep = () => papers().get(keepId());
  const merged = () =>
    mergeIds()
      .map((id) => papers().get(id))
      .filter((p): p is Paper => !!p);
  const moves = () => {
    const k = keep();
    return k ? mergeMoves(k, merged()) : [];
  };
  const others = () => {
    const list = mergeIds();
    return list.length === 1 ? `"${paperTitle(papers().get(list[0]!))}"` : `${list.length} others`;
  };

  const confirm = async () => {
    setBusy(true);
    const result = await store.mergePapers(keepId(), mergeIds());
    setBusy(false);
    if (result) props.onDone?.();
  };

  return (
    <div class="merge-confirm" role="group" aria-label="Merge items">
      <p class="merge-confirm-text">
        Keep "<MathText text={paperTitle(keep())} />" and fold <MathText text={others()} /> into it.{' '}
        <Show when={moves().length > 0} fallback={<>Nothing else moves.</>}>
          Moves: {moves().join(', ')}.
        </Show>{' '}
        Files on disk stay where they are; Split undoes it.
      </p>
      <div class="merge-confirm-actions">
        <button
          type="button"
          class="secondary-btn"
          disabled={busy() || !keep()}
          onClick={() => void confirm()}
        >
          {busy() ? 'Merging...' : 'Merge'}
        </button>
        <button
          type="button"
          class="paper-verdict-link"
          disabled={busy()}
          onClick={() => setKeepIndex((i) => (i + 1) % props.ids.length)}
        >
          {props.ids.length === 2 ? 'Keep the other one instead' : 'Keep another one instead'}
        </button>
        <span class="merge-confirm-spacer" />
        <button
          type="button"
          class="paper-mini-btn"
          disabled={busy()}
          onClick={() => props.onCancel()}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

/** "Possibly the same work as: …", under the verdict, for a record the
 * engine groups with others. Each other title selects that record. */
export function DuplicateLine(props: { paper: Paper }) {
  const store = projectsStore;
  const match = () => store.state.duplicates[props.paper.id];
  const others = () => match()?.others ?? [];
  const papers = usePapers(others);
  const [confirming, setConfirming] = createSignal(false);
  createEffect(() => {
    void props.paper.id;
    setConfirming(false);
  });
  const dismiss = () => {
    void engine
      .libraryNotDuplicates([props.paper.id, ...others()])
      .catch((error) => projectsStore.reportError(error));
  };

  return (
    <Show when={match()}>
      {(m) => (
        <div class="paper-dup">
          <Show
            when={confirming()}
            fallback={
              <div class="paper-dup-line">
                <span class="paper-dup-text">
                  Possibly the same work as:{' '}
                  <For each={m().others}>
                    {(id, i) => (
                      <>
                        {i() > 0 ? ', ' : ''}
                        <button
                          type="button"
                          class="paper-verdict-link"
                          title="Show this item"
                          onClick={() => store.select(id)}
                        >
                          <MathText text={paperTitle(papers().get(id), 'another item')} />
                        </button>
                      </>
                    )}
                  </For>{' '}
                  ({m().reason})
                </span>
                <button
                  type="button"
                  class="paper-mini-btn"
                  title="Fold the other record into this one"
                  onClick={() => setConfirming(true)}
                >
                  Merge
                </button>
                <Show when={CAN_DISMISS_DUPLICATES}>
                  <button type="button" class="paper-mini-btn" onClick={dismiss}>
                    Not the same
                  </button>
                </Show>
              </div>
            }
          >
            <MergeConfirm
              ids={[props.paper.id, ...m().others]}
              onCancel={() => setConfirming(false)}
              onDone={() => setConfirming(false)}
            />
          </Show>
        </div>
      )}
    </Show>
  );
}

/** "Merged from 3 items", with the way back. */
export function MergedLine(props: { paper: Paper }) {
  const store = projectsStore;
  const [busy, setBusy] = createSignal(false);
  const split = async () => {
    setBusy(true);
    await store.unmerge(props.paper.id);
    setBusy(false);
  };
  return (
    <Show when={props.paper.mergedCount > 0}>
      <div class="paper-dup-line paper-merged">
        <span class="paper-dup-text">
          Merged from {props.paper.mergedCount + 1} items
          <Show when={(props.paper.autoMerged ?? []).length > 0}>
            {' '}
            <span class="paper-auto-merged">
              Merged automatically ({props.paper.autoMerged.join('; ')})
            </span>
          </Show>
        </span>
        <button
          type="button"
          class="paper-mini-btn"
          disabled={busy()}
          title="Restore the records as they were before merging"
          onClick={() => void split()}
        >
          Split
        </button>
      </div>
    </Show>
  );
}
