/** The graph's View popover: how busy the graph is drawn. Edges, labels,
 * items with no edge, and the two kinds of ghost with their thresholds.
 * Every choice goes through the store, which keeps them across sessions
 * (graphViewPrefs.ts). Escape or a press outside closes it; focus starts
 * on its first control and goes back to the View button on Escape. */
import { For, onMount, Show } from 'solid-js';
import MathText from '../../components/MathText';
import {
  CITED_WITH_MAX,
  CITED_WITH_MIN,
  MISSING_MAX,
  MISSING_MIN,
  projectsStore,
} from './projectsStore';

interface Choice<T extends string> {
  value: T;
  label: string;
  title?: string;
}

function Segmented<T extends string>(props: {
  label: string;
  value: T;
  choices: Choice<T>[];
  onPick: (value: T) => void;
}) {
  return (
    <>
      <span class="graph-view-menu-label">{props.label}</span>
      <div class="projects-launch-toggle" role="radiogroup" aria-label={props.label}>
        <For each={props.choices}>
          {(choice) => (
            <button
              type="button"
              role="radio"
              aria-checked={props.value === choice.value}
              classList={{ 'is-active': props.value === choice.value }}
              title={choice.title}
              onClick={() => props.onPick(choice.value)}
            >
              {choice.label}
            </button>
          )}
        </For>
      </div>
    </>
  );
}

function Stepper(props: {
  group: string;
  fewer: string;
  more: string;
  text: string;
  value: number;
  min: number;
  max: number;
  onStep: (value: number) => void;
}) {
  return (
    <span class="graph-control graph-stepper" role="group" aria-label={props.group}>
      <button
        type="button"
        aria-label={props.fewer}
        disabled={props.value <= props.min}
        onClick={() => props.onStep(props.value - 1)}
      >
        -
      </button>
      <span class="graph-stepper-value">
        <MathText text={props.text} />
      </span>
      <button
        type="button"
        aria-label={props.more}
        disabled={props.value >= props.max}
        onClick={() => props.onStep(props.value + 1)}
      >
        +
      </button>
    </span>
  );
}

export default function GraphViewMenu(props: {
  ref?: (el: HTMLDivElement) => void;
  /** The cited-with row is a project's alone. */
  inProject: boolean;
  onClose: (refocus?: boolean) => void;
}) {
  const store = projectsStore;
  const prefs = () => store.state.viewPrefs;
  let el: HTMLDivElement | undefined;
  onMount(() => el?.querySelector<HTMLElement>('button')?.focus());
  return (
    <div
      ref={(node) => {
        el = node;
        props.ref?.(node);
      }}
      class="graph-view-menu"
      role="dialog"
      aria-label="Graph view"
      onKeyDown={(e) => {
        if (e.key !== 'Escape') return;
        e.preventDefault();
        e.stopPropagation();
        props.onClose(true);
      }}
    >
      <Segmented
        label="Edges"
        value={prefs().edges}
        choices={[
          { value: 'all', label: 'All' },
          { value: 'selected', label: 'Selected', title: 'Only the edges of the item in hand' },
        ]}
        onPick={(v) => store.setEdgeMode(v)}
      />
      <Segmented
        label="Labels"
        value={prefs().labels}
        choices={[
          { value: 'many', label: 'Many', title: 'As many as fit, more as you zoom in' },
          { value: 'few', label: 'Few', title: 'The most cited only' },
          { value: 'hover', label: 'Hover', title: 'None until hovered or selected' },
        ]}
        onPick={(v) => store.setLabelMode(v)}
      />
      <Segmented
        label="Unlinked items"
        value={prefs().unlinked}
        choices={[
          { value: 'hide', label: 'Hide', title: 'Count items with no citations here' },
          { value: 'show', label: 'Show', title: 'Draw items with no citations here too' },
        ]}
        onPick={(v) => store.setUnlinkedMode(v)}
      />
      <span class="graph-view-menu-label">Missing works</span>
      <div class="graph-view-menu-row">
        <button
          type="button"
          class="graph-control is-text"
          aria-pressed={store.state.missing.on}
          classList={{ 'is-on': store.state.missing.on }}
          title="Show works several items here cite that are not in the library"
          onClick={() => store.setMissing(!store.state.missing.on)}
        >
          Missing
        </button>
        <Show when={store.state.missing.on}>
          <Stepper
            group="Cited by at least"
            fewer="Fewer citers"
            more="More citers"
            text={`cited by $\\geq ${store.state.missing.min}$`}
            value={store.state.missing.min}
            min={MISSING_MIN}
            max={MISSING_MAX}
            onStep={(v) => store.setMissingMin(v)}
          />
        </Show>
      </div>
      <Show when={props.inProject}>
        <span class="graph-view-menu-label">Cited with</span>
        <div class="graph-view-menu-row">
          <button
            type="button"
            class="graph-control is-text"
            aria-pressed={store.state.citedWith.on}
            classList={{ 'is-on': store.state.citedWith.on }}
            title="Show works the library often cites alongside these items"
            onClick={() => store.setCitedWith(!store.state.citedWith.on)}
          >
            Cited with
          </button>
          <Show when={store.state.citedWith.on}>
            <Stepper
              group="Cited with at least"
              fewer="Fewer members"
              more="More members"
              text={`with $\\geq ${store.state.citedWith.min}$ of these`}
              value={store.state.citedWith.min}
              min={CITED_WITH_MIN}
              max={CITED_WITH_MAX}
              onStep={(v) => store.setCitedWithMin(v)}
            />
          </Show>
        </div>
      </Show>
    </div>
  );
}
