/** The middle column under Health: what the engine suspects about the
 * items (`library_health`), grouped, each finding one row with a reason
 * and its fixes as quiet text buttons on the right.
 *
 * A row selects its item into the detail column; double-click opens it.
 * Every fix goes through a command that already exists - a merge through
 * the same inline confirmation the list uses, a year or a title as a user
 * edit - and Ignore records the finding so it never comes back. Nothing
 * here goes online unless the user turned lookups on and presses Look up. */
import { createSignal, For, Show } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import { settings } from '../../stores/settings';
import type { HealthFinding } from '../../types/engine';
import { MergeConfirm } from './MergeConfirm';
import { openPaper } from './openPaper';
import { projectsStore } from './projectsStore';
import MathText from '../../components/MathText';

/** Finding kind and item: one row's identity, for the open merge. */
const rowKey = (f: HealthFinding) => `${f.kind}:${f.paperId}`;

export default function HealthList() {
  const store = projectsStore;
  const [merging, setMerging] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);

  const run = async (finding: HealthFinding, work: () => Promise<unknown>) => {
    setBusy(rowKey(finding));
    try {
      await work();
    } finally {
      setBusy(null);
    }
  };

  const lookUp = (finding: HealthFinding) =>
    run(finding, async () => {
      try {
        const paper = await engine.libraryPaper(finding.paperId);
        const mode = paper.doi || paper.arxiv || paper.isbn ? 'id' : 'title';
        await engine.libraryLookup(finding.paperId, mode);
      } catch (error) {
        store.reportError(error);
      }
      store.select(finding.paperId);
    });

  const actions = (finding: HealthFinding) => {
    const out: { label: string; title?: string; run: () => void }[] = [];
    const suggestion = finding.suggestion;
    switch (finding.kind) {
      case 'duplicate':
        out.push({
          label: 'Merge...',
          title: 'Fold these records into one',
          run: () => setMerging(merging() === rowKey(finding) ? null : rowKey(finding)),
        });
        break;
      case 'year':
        if (suggestion) {
          out.push({
            label: `Use ${suggestion}`,
            title: 'Set the year, as your own edit',
            run: () =>
              void run(finding, () =>
                store.updatePaper(finding.paperId, { year: Number(suggestion) })
              ),
          });
        }
        break;
      case 'title':
        if (suggestion) {
          out.push({
            label: 'Use page title',
            title: suggestion,
            run: () =>
              void run(finding, () => store.updatePaper(finding.paperId, { title: suggestion })),
          });
        }
        break;
      case 'weak':
        if (settings.onlineLookup) {
          out.push({
            label: 'Look up',
            title: 'Fetch metadata online for this item',
            run: () => void lookUp(finding),
          });
        }
        break;
      case 'missing':
        break;
    }
    out.push({
      label: 'Show',
      title: 'Show the item in the details',
      run: () => store.select(finding.paperId),
    });
    out.push({
      label: 'Ignore',
      title: 'Do not mention this again',
      run: () => void run(finding, () => store.ignoreFinding(finding.paperId, finding.kind)),
    });
    return out;
  };

  return (
    <div class="projects-col-scroll paper-list health-list" role="list" aria-label="Health">
      <For each={store.state.health}>
        {(group) => (
          <>
            <div class="paper-group-head" role="presentation">
              <span class="paper-group-name">{group.label}</span>
              <span class="paper-group-count">{group.findings.length}</span>
            </div>
            <For each={group.findings}>
              {(finding) => (
                <div role="listitem" data-health={finding.kind}>
                  <div
                    class="paper-row"
                    classList={{ 'is-selected': store.state.selectedId === finding.paperId }}
                    onClick={() => store.select(finding.paperId)}
                    onDblClick={() => void openPaper(finding.paperId)}
                  >
                    <span class="paper-row-main">
                      <MathText class="paper-row-title" text={finding.title} />
                      <span class="paper-row-meta" title={finding.reason}>
                        {finding.reason}
                      </span>
                    </span>
                    <span class="health-actions">
                      <For each={actions(finding)}>
                        {(action) => (
                          <button
                            type="button"
                            class="health-action"
                            title={action.title}
                            disabled={busy() === rowKey(finding)}
                            onClick={(e) => {
                              e.stopPropagation();
                              action.run();
                            }}
                          >
                            {action.label}
                          </button>
                        )}
                      </For>
                    </span>
                  </div>
                  <Show when={merging() === rowKey(finding)}>
                    <div class="health-merge">
                      <MergeConfirm
                        ids={finding.paperIds}
                        onCancel={() => setMerging(null)}
                        onDone={() => setMerging(null)}
                      />
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </>
        )}
      </For>
    </div>
  );
}
