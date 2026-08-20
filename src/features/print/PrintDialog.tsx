/** The print dialog: what will print on the left, how it prints on the right.
 *
 * The preview is the document that is about to be sent — the temp PDF the
 * engine built — rendered through the ordinary pdfr:// protocol. That is what
 * makes it worth looking at: reordering, rotation and markup are already baked
 * into those bytes, so this is the output rather than an impression of it. */
import { createEffect, createMemo, createSignal, For, Show } from 'solid-js';
import { renderUrl } from '../../lib/rendering/renderSource';
import { checkPageRange } from './pageRange';
import { choiceLabel, optionLabel, SHOWN_OPTIONS } from './printLabels';
import {
  closePrint,
  exportAsPdf,
  loadOptions,
  printState,
  setIncludeAnnotations,
  submitPrint,
} from './printStore';

/** The destination that is not a printer. Kept as a value no CUPS name can
 * take, so it can never be mistaken for one on the way to `lp`. */
const SAVE_AS_PDF = ' pdf';

export default function PrintDialog() {
  const [destination, setDestination] = createSignal('');
  const [copies, setCopies] = createSignal(1);
  const [allPages, setAllPages] = createSignal(true);
  const [range, setRange] = createSignal('');
  const [chosen, setChosen] = createSignal<Record<string, string>>({});
  const [page, setPage] = createSignal(0);

  createEffect(() => {
    const printers = printState.printers;
    if (destination() !== '' || printers.length === 0) return;
    setDestination((printers.find((p) => p.isDefault) ?? printers[0]!).name);
  });

  createEffect(() => {
    const defaults: Record<string, string> = {};
    for (const option of printState.options) defaults[option.key] = option.default;
    setChosen(defaults);
  });

  const toPdf = () => destination() === SAVE_AS_PDF;
  const pageCount = () => printState.pageCount;
  const rangeProblem = () => (allPages() ? null : checkPageRange(range(), pageCount()));
  const ready = () => !printState.preparing && printState.previewDocId >= 0;
  const canGo = () =>
    ready() && !printState.submitting && destination() !== '' && rangeProblem() === null;

  const options = createMemo(() =>
    printState.options
      .filter((option) => SHOWN_OPTIONS.includes(option.key))
      .sort((a, b) => SHOWN_OPTIONS.indexOf(a.key) - SHOWN_OPTIONS.indexOf(b.key))
  );

  // Clamp, because a rebuild can change the page count underneath us.
  const shown = () => Math.min(page(), Math.max(0, pageCount() - 1));
  const previewUrl = () =>
    printState.previewDocId < 0
      ? null
      : renderUrl({
          docId: printState.previewDocId,
          srcIndex: shown(),
          rotation: 0,
          scaleMilli: 1500,
          generation: printState.previewGeneration,
          kind: 'page',
        });

  const step = (by: number) =>
    setPage(() => Math.min(Math.max(0, shown() + by), Math.max(0, pageCount() - 1)));

  const go = () => {
    const pages = allPages() ? null : range().trim();
    if (toPdf()) return void exportAsPdf(pages);
    void submitPrint(destination(), copies(), pages, Object.entries(chosen()));
  };

  return (
    <div class="print-dialog">
      <div class="print-preview">
        <div class="print-sheet">
          <Show when={previewUrl()} fallback={<div class="print-sheet-empty" />}>
            <img
              src={previewUrl()!}
              alt={`Page ${shown() + 1}`}
              draggable={false}
              classList={{ 'is-stale': printState.preparing }}
            />
          </Show>
        </div>
        <div class="print-pager">
          <button
            type="button"
            aria-label="Previous page"
            onClick={() => step(-1)}
            disabled={shown() === 0}
          >
            &lsaquo;
          </button>
          <span>
            <Show when={ready()} fallback="Preparing…">
              Page {shown() + 1} of {pageCount()}
            </Show>
          </span>
          <button
            type="button"
            aria-label="Next page"
            onClick={() => step(1)}
            disabled={shown() >= pageCount() - 1}
          >
            &rsaquo;
          </button>
        </div>
      </div>

      <div class="print-settings">
        <h2>Print</h2>

        <label class="print-field">
          <span>Destination</span>
          <select
            value={destination()}
            onChange={(e) => {
              setDestination(e.currentTarget.value);
              if (e.currentTarget.value !== SAVE_AS_PDF) void loadOptions(e.currentTarget.value);
            }}
          >
            <For each={printState.printers}>
              {(p) => <option value={p.name}>{p.name.replaceAll('_', ' ')}</option>}
            </For>
            <option value={SAVE_AS_PDF}>Save as PDF…</option>
          </select>
        </label>

        <div class="print-row">
          <Show when={!toPdf()}>
            <label class="print-field print-copies">
              <span>Copies</span>
              <input
                type="number"
                min="1"
                max="99"
                value={copies()}
                onInput={(e) =>
                  setCopies(Math.min(99, Math.max(1, Number(e.currentTarget.value) || 1)))
                }
              />
            </label>
          </Show>
          <div class="print-field print-pages">
            <span>Pages</span>
            <div class="print-pages-controls">
              <label class="print-choice">
                <input
                  type="radio"
                  name="print-pages"
                  checked={allPages()}
                  onChange={() => setAllPages(true)}
                />
                <span>All</span>
              </label>
              <label class="print-choice">
                <input
                  type="radio"
                  name="print-pages"
                  checked={!allPages()}
                  onChange={() => setAllPages(false)}
                />
                <input
                  type="text"
                  class="print-range-input"
                  placeholder="1-4, 9"
                  value={range()}
                  onFocus={() => setAllPages(false)}
                  onInput={(e) => setRange(e.currentTarget.value)}
                />
              </label>
            </div>
          </div>
        </div>
        <Show when={rangeProblem()}>
          {(problem) => <p class="print-problem">{problem().message}</p>}
        </Show>

        <Show when={!toPdf()}>
          <For each={options()}>
            {(option) => (
              <label class="print-field">
                <span>{optionLabel(option.key, option.label)}</span>
                <select
                  value={chosen()[option.key] ?? option.default}
                  onChange={(e) => setChosen({ ...chosen(), [option.key]: e.currentTarget.value })}
                >
                  <For each={option.choices}>
                    {(choice) => <option value={choice}>{choiceLabel(option.key, choice)}</option>}
                  </For>
                </select>
              </label>
            )}
          </For>
        </Show>

        <Show when={printState.hasAnnotations}>
          <label class="print-toggle">
            <input
              type="checkbox"
              checked={printState.includeAnnotations}
              disabled={printState.preparing}
              onChange={(e) => void setIncludeAnnotations(e.currentTarget.checked)}
            />
            <span>
              Include my highlights and notes
              <small>Off prints the document as its author wrote it.</small>
            </span>
          </label>
        </Show>

        <div class="print-actions">
          <button type="button" class="secondary-btn" onClick={() => void closePrint()}>
            Cancel
          </button>
          <button
            type="button"
            class="primary-btn"
            disabled={!canGo()}
            ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
            onClick={go}
          >
            <Show when={!printState.submitting} fallback={toPdf() ? 'Saving…' : 'Printing…'}>
              {toPdf() ? 'Save…' : 'Print'}
            </Show>
          </button>
        </div>
      </div>
    </div>
  );
}
