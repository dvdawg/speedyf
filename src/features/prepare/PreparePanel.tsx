/** The Form panel while preparing a form: auto-detect and its proposals,
 * the selected field's properties, the page's Tab order, and the form as a
 * whole (flatten, data in and out).
 *
 * Every change is a `setFormDesign` edit, undoable like any other; typing
 * into one box is one undo step. */
import { createMemo, createSignal, For, Show, useContext, type JSX } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose } from '../../components/icons';
import { TabContext } from '../../app/TabContext';
import type { FormOption } from '../../types/engine';
import type { PageId } from '../../types/model';
import { userDatePicture } from '../forms/formBehaviour';
import { exportFormData, importFormData } from '../forms/formData';
import {
  deleteFields,
  duplicateFields,
  kindLabel,
  moveInTabOrder,
  nameProblem,
  patchFields,
  setTabOrder,
  tabOrderOf,
  type DesignField,
  type DesignProps,
} from './model';
import {
  CALC_OPS,
  DATE_PICTURES,
  FORMAT_CHOICES,
  TIME_PICTURES,
  calcFor,
  choiceOf,
  formatFor,
  type FormatChoice,
} from './formats';
import {
  acceptProposals,
  commitDesign,
  designOf,
  detectFields,
  prepareState,
  rejectProposals,
  select,
  selectedIds,
  setShowTabs,
  stopPreparing,
} from './prepareStore';

export default function PreparePanel() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;
  const design = () => designOf(tab);
  const selected = createMemo(() => {
    const ids = selectedIds(tab);
    return design().fields.filter((f) => ids.includes(f.id));
  });
  const one = () => (selected().length === 1 ? selected()[0]! : null);

  /** One undo step per box being typed in. */
  let group: { key: string; symbol: symbol } | null = null;
  const groupFor = (key: string) => {
    if (group?.key !== key) group = { key, symbol: Symbol(key) };
    return group.symbol;
  };
  const patch = (
    p: Partial<DesignProps> | ((f: DesignField) => Partial<DesignProps>),
    key?: string
  ) => {
    const ids = selected().map((f) => f.id);
    if (ids.length === 0) return;
    commitDesign(
      tab,
      patchFields(design(), ids, typeof p === 'function' ? p : () => p),
      key ? groupFor(`${ids.join(',')}:${key}`) : undefined
    );
  };

  /** The page whose Tab order is shown: the selected field's, else the one
   * in view. */
  const tabPage = (): PageId | null => {
    const f = selected()[0];
    if (f) return f.pageId;
    return doc.pages[tab.viewport.state.currentPage]?.id ?? doc.pages[0]?.id ?? null;
  };
  const tabPageNumber = () => {
    const id = tabPage();
    return id ? doc.pages.findIndex((p) => p.id === id) + 1 : 0;
  };
  const order = createMemo(() => {
    const id = tabPage();
    if (!id) return [] as DesignField[];
    const byId = new Map(design().fields.map((f) => [f.id, f]));
    return tabOrderOf(design(), id)
      .map((x) => byId.get(x))
      .filter((f): f is DesignField => !!f);
  });

  return (
    <div class="form-panel prepare-panel" aria-label="Prepare form">
      <div class="panel-head">
        <h2>Prepare form</h2>
        <IconButton label="Done preparing the form" onClick={() => stopPreparing()}>
          <IconClose />
        </IconButton>
      </div>
      <div class="prepare-scroll">
        <section class="prepare-section">
          <div class="prepare-row">
            <button
              type="button"
              class="secondary-btn"
              disabled={prepareState.detecting}
              title="Propose fields from the lines, boxes and checkboxes printed on every page"
              onClick={() => void detectFields(tab)}
            >
              {prepareState.detecting ? 'Detecting...' : 'Auto-detect fields'}
            </button>
          </div>
          <Show when={prepareState.proposals.length > 0}>
            <div class="prepare-row prepare-proposals-bar">
              <span>{prepareState.proposals.length} proposed</span>
              <button type="button" class="secondary-btn" onClick={() => acceptProposals(tab)}>
                Accept all
              </button>
              <button type="button" class="secondary-btn" onClick={() => rejectProposals()}>
                Reject all
              </button>
            </div>
          </Show>
        </section>

        <section class="prepare-section" aria-label="Field properties">
          <Show
            when={selected().length > 0}
            fallback={
              <p class="panel-note">
                {design().fields.length === 0
                  ? 'No fields yet. Draw one with the tools above, or auto-detect them.'
                  : `${design().fields.length} field${design().fields.length === 1 ? '' : 's'}. Click one on the page to edit it.`}
              </p>
            }
          >
            <Show
              when={one()}
              fallback={
                <MultiProps count={selected().length} first={selected()[0]!} patch={patch} />
              }
            >
              {(f) => <FieldProps field={f()} patch={patch} />}
            </Show>
            <div class="prepare-row">
              <button
                type="button"
                class="secondary-btn"
                onClick={() => {
                  const { design: next, ids } = duplicateFields(
                    design(),
                    selected().map((f) => f.id)
                  );
                  commitDesign(tab, next);
                  select(ids);
                }}
              >
                Duplicate
              </button>
              <button
                type="button"
                class="secondary-btn prepare-danger"
                onClick={() => {
                  commitDesign(
                    tab,
                    deleteFields(
                      design(),
                      selected().map((f) => f.id)
                    )
                  );
                  select([]);
                }}
              >
                Delete
              </button>
            </div>
          </Show>
        </section>

        <section class="prepare-section" aria-label="Tab order">
          <h3 class="prepare-head">Tab order, page {tabPageNumber()}</h3>
          <label class="prepare-check">
            <input
              type="checkbox"
              checked={prepareState.showTabs}
              onInput={(e) => setShowTabs(e.currentTarget.checked)}
            />
            Show numbers on the page
          </label>
          <div class="prepare-row">
            <button
              type="button"
              class="secondary-btn"
              onClick={() => {
                const id = tabPage();
                if (id) commitDesign(tab, setTabOrder(design(), id, 'R'));
              }}
            >
              Order by rows
            </button>
            <button
              type="button"
              class="secondary-btn"
              onClick={() => {
                const id = tabPage();
                if (id) commitDesign(tab, setTabOrder(design(), id, 'C'));
              }}
            >
              Order by columns
            </button>
          </div>
          <TabList
            fields={order()}
            selected={selected().map((f) => f.id)}
            onSelect={(id) => select([id])}
            onMove={(id, to) => {
              const page = tabPage();
              if (page) commitDesign(tab, moveInTabOrder(design(), page, id, to));
            }}
          />
        </section>

        <section class="prepare-section" aria-label="Form">
          <label
            class="prepare-check"
            title="Write the fields into the pages when the document is saved, so they can no longer be filled"
          >
            <input
              type="checkbox"
              checked={design().flatten}
              onInput={(e) => commitDesign(tab, { ...design(), flatten: e.currentTarget.checked })}
            />
            Flatten form on save
          </label>
          <div class="prepare-row">
            <button type="button" class="secondary-btn" onClick={() => void exportFormData(tab)}>
              Export data...
            </button>
            <button type="button" class="secondary-btn" onClick={() => void importFormData(tab)}>
              Import data...
            </button>
          </div>
        </section>
      </div>
      <div class="form-nav-foot">
        <span>Fields are written into the PDF when you save.</span>
        <button type="button" class="secondary-btn" onClick={() => stopPreparing()}>
          Done
        </button>
      </div>
    </div>
  );
}

type Patch = (
  p: Partial<DesignProps> | ((f: DesignField) => Partial<DesignProps>),
  key?: string
) => void;

function Field(props: { label: string; children: JSX.Element; hint?: string | null }) {
  return (
    <label class="prepare-field-row">
      <span class="prepare-label">{props.label}</span>
      {props.children}
      <Show when={props.hint}>
        <span class="prepare-hint">{props.hint}</span>
      </Show>
    </label>
  );
}

function Check(props: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <label class="prepare-check" title={props.title}>
      <input
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onInput={(e) => props.onChange(e.currentTarget.checked)}
      />
      {props.label}
    </label>
  );
}

function FontSize(props: { value: number; onChange: (v: number) => void }) {
  return (
    <Field label="Font size">
      <input
        class="prepare-input prepare-narrow"
        type="number"
        min="0"
        max="72"
        step="0.5"
        placeholder="Auto"
        aria-label="Font size in points, empty for auto"
        value={props.value > 0 ? props.value : ''}
        onInput={(e) => {
          const v = parseFloat(e.currentTarget.value);
          props.onChange(Number.isFinite(v) && v > 0 ? Math.min(72, v) : 0);
        }}
      />
    </Field>
  );
}

function Align(props: { value: number; onChange: (v: 0 | 1 | 2) => void }) {
  const names = ['Left', 'Centre', 'Right'] as const;
  return (
    <div class="prepare-field-row" role="radiogroup" aria-label="Alignment">
      <span class="prepare-label">Alignment</span>
      <div class="prepare-segmented">
        <For each={names}>
          {(n, i) => (
            <button
              type="button"
              role="radio"
              aria-checked={props.value === i()}
              classList={{ 'is-active': props.value === i() }}
              onClick={() => props.onChange(i() as 0 | 1 | 2)}
            >
              {n}
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

function MultiProps(props: { count: number; first: DesignField; patch: Patch }) {
  return (
    <div class="prepare-props">
      <p class="prepare-kind">{props.count} fields selected</p>
      <Check
        label="Required"
        checked={props.first.required}
        onChange={(v) => props.patch({ required: v })}
      />
      <Check
        label="Read-only"
        checked={props.first.readOnly}
        onChange={(v) => props.patch({ readOnly: v })}
      />
      <FontSize
        value={props.first.fontSize}
        onChange={(v) => props.patch({ fontSize: v }, 'font')}
      />
      <Align value={props.first.align} onChange={(v) => props.patch({ align: v })} />
    </div>
  );
}

function FieldProps(props: { field: DesignField; patch: Patch }) {
  const tab = useContext(TabContext)!;
  const f = () => props.field;
  const [nameDraft, setNameDraft] = createSignal<string | null>(null);
  const problem = () => {
    const draft = nameDraft();
    return draft === null ? null : nameProblem(designOf(tab), f().id, draft);
  };
  const isText = () => f().kind === 'text';
  const isChoice = () => f().kind === 'combo' || f().kind === 'list';
  const isButton = () => f().kind === 'checkbox' || f().kind === 'radio';
  const otherTextFields = createMemo(() => {
    const names = new Set<string>();
    for (const g of designOf(tab).fields) {
      if (g.kind === 'text' && g.name !== f().name) names.add(g.name);
    }
    return [...names];
  });

  const rename = (value: string) => {
    setNameDraft(value);
    if (nameProblem(designOf(tab), f().id, value)) return;
    const old = f().name;
    const name = value.trim();
    if (f().kind === 'radio' && f().origin) {
      // A group's name is its field's: rename every button of it.
      const ids = designOf(tab)
        .fields.filter((g) => g.kind === 'radio' && g.name === old)
        .map((g) => g.id);
      commitDesign(
        tab,
        patchFields(designOf(tab), ids, () => ({ name })),
        Symbol.for(`name:${f().id}`)
      );
      return;
    }
    props.patch({ name }, 'name');
  };

  return (
    <div class="prepare-props">
      <p class="prepare-kind">{kindLabel(f())}</p>
      <Field label={f().kind === 'radio' ? 'Group name' : 'Name'} hint={problem()}>
        <input
          class="prepare-input"
          classList={{ 'has-problem': problem() !== null }}
          value={nameDraft() ?? f().name}
          onInput={(e) => rename(e.currentTarget.value)}
          onBlur={() => setNameDraft(null)}
        />
      </Field>
      <Field label="Tooltip">
        <input
          class="prepare-input"
          value={f().tooltip}
          placeholder="What the field asks for"
          onInput={(e) => props.patch({ tooltip: e.currentTarget.value }, 'tooltip')}
        />
      </Field>
      <div class="prepare-checks">
        <Check
          label="Required"
          checked={f().required}
          onChange={(v) => props.patch({ required: v })}
        />
        <Check
          label="Read-only"
          checked={f().readOnly}
          onChange={(v) => props.patch({ readOnly: v })}
        />
        <Check label="Border" checked={f().border} onChange={(v) => props.patch({ border: v })} />
      </div>

      <Show when={isButton()}>
        <Field label="Export value">
          <input
            class="prepare-input"
            value={f().exportValue}
            onInput={(e) => props.patch({ exportValue: e.currentTarget.value.trim() }, 'export')}
          />
        </Field>
        <Check
          label={f().kind === 'radio' ? 'Selected by default' : 'Checked by default'}
          checked={f().defaultValue !== '' && f().defaultValue === f().exportValue}
          onChange={(v) => {
            const value = v ? f().exportValue : '';
            if (f().kind === 'radio') {
              // The group's default is one button's export value.
              const ids = designOf(tab)
                .fields.filter((g) => g.kind === 'radio' && g.name === f().name)
                .map((g) => g.id);
              commitDesign(
                tab,
                patchFields(designOf(tab), ids, () => ({ defaultValue: value }))
              );
            } else {
              props.patch({ defaultValue: value });
            }
          }}
        />
      </Show>

      <Show when={isText()}>
        <Field label="Default value">
          <input
            class="prepare-input"
            value={f().defaultValue}
            onInput={(e) => props.patch({ defaultValue: e.currentTarget.value }, 'default')}
          />
        </Field>
        <FontSize value={f().fontSize} onChange={(v) => props.patch({ fontSize: v }, 'font')} />
        <Align value={f().align} onChange={(v) => props.patch({ align: v })} />
        <Field label="Max length">
          <input
            class="prepare-input prepare-narrow"
            type="number"
            min="0"
            placeholder="None"
            value={f().maxLen > 0 ? f().maxLen : ''}
            onInput={(e) => {
              const v = parseInt(e.currentTarget.value, 10);
              const maxLen = Number.isFinite(v) && v > 0 ? v : 0;
              props.patch({ maxLen, ...(maxLen === 0 ? { comb: false } : {}) }, 'maxlen');
            }}
          />
        </Field>
        <div class="prepare-checks">
          <Check
            label="Multiline"
            checked={f().multiline}
            onChange={(v) => props.patch({ multiline: v, ...(v ? { comb: false } : {}) })}
          />
          <Check
            label="Comb"
            title="One character per cell, across Max length cells"
            disabled={f().maxLen === 0 || f().multiline}
            checked={f().comb}
            onChange={(v) => props.patch({ comb: v })}
          />
        </div>
        <FormatProps field={f()} patch={props.patch} />
        <CalcProps field={f()} patch={props.patch} others={otherTextFields()} />
      </Show>

      <Show when={isChoice()}>
        <FontSize value={f().fontSize} onChange={(v) => props.patch({ fontSize: v }, 'font')} />
        <OptionsEditor field={f()} patch={props.patch} />
        <Field label="Default">
          <select
            class="prepare-input"
            value={f().defaultValue}
            onChange={(e) => props.patch({ defaultValue: e.currentTarget.value })}
          >
            <option value="">None</option>
            <For each={f().options}>{(o) => <option value={o.export}>{o.label}</option>}</For>
          </select>
        </Field>
      </Show>
    </div>
  );
}

function FormatProps(props: { field: DesignField; patch: Patch }) {
  const f = () => props.field;
  const choice = () => choiceOf(f().format);
  const format = () => f().format;
  return (
    <>
      <Field label="Format">
        <select
          class="prepare-input"
          value={choice()}
          onChange={(e) =>
            props.patch({
              format: formatFor(e.currentTarget.value as FormatChoice, userDatePicture()),
            })
          }
        >
          <For each={FORMAT_CHOICES}>{(c) => <option value={c.value}>{c.label}</option>}</For>
        </select>
      </Field>
      <Show when={f().opaqueScripts && choice() === 'none'}>
        <p class="prepare-hint">
          This field has a script SpeedyF does not read; choosing a format replaces it.
        </p>
      </Show>
      <Show when={format()?.kind === 'number' || format()?.kind === 'percent'}>
        <Field label="Decimals">
          <input
            class="prepare-input prepare-narrow"
            type="number"
            min="0"
            max="10"
            value={(format() as { decimals: number }).decimals}
            onInput={(e) => {
              const v = Math.max(0, Math.min(10, parseInt(e.currentTarget.value, 10) || 0));
              props.patch(
                (g) => ({
                  format:
                    g.format?.kind === 'number' || g.format?.kind === 'percent'
                      ? { ...g.format, decimals: v }
                      : g.format,
                }),
                'decimals'
              );
            }}
          />
        </Field>
      </Show>
      <Show when={format()?.kind === 'number' && choice() === 'currency'}>
        <Field label="Symbol">
          <input
            class="prepare-input prepare-narrow"
            value={(format() as { currency: string }).currency}
            onInput={(e) => {
              const currency = e.currentTarget.value || '$';
              props.patch(
                (g) => ({
                  format: g.format?.kind === 'number' ? { ...g.format, currency } : g.format,
                }),
                'currency'
              );
            }}
          />
        </Field>
      </Show>
      <Show when={format()?.kind === 'date'}>
        <Field label="Date format">
          <select
            class="prepare-input"
            value={(format() as { format: string }).format}
            onChange={(e) =>
              props.patch({ format: { kind: 'date', format: e.currentTarget.value } })
            }
          >
            <For each={[...new Set([(format() as { format: string }).format, ...DATE_PICTURES])]}>
              {(p) => <option value={p}>{p}</option>}
            </For>
          </select>
        </Field>
      </Show>
      <Show when={format()?.kind === 'time'}>
        <Field label="Time format">
          <select
            class="prepare-input"
            value={(format() as { format: string }).format}
            onChange={(e) =>
              props.patch({ format: { kind: 'time', format: e.currentTarget.value } })
            }
          >
            <For each={[...new Set([(format() as { format: string }).format, ...TIME_PICTURES])]}>
              {(p) => <option value={p}>{p}</option>}
            </For>
          </select>
        </Field>
      </Show>
      <Show when={format()?.kind === 'mask'}>
        <Field label="Mask" hint="9 digit, A letter, O either, X any">
          <input
            class="prepare-input"
            value={(format() as { mask: string }).mask}
            onInput={(e) =>
              props.patch({ format: { kind: 'mask', mask: e.currentTarget.value || '9' } }, 'mask')
            }
          />
        </Field>
      </Show>
    </>
  );
}

function CalcProps(props: { field: DesignField; patch: Patch; others: string[] }) {
  const f = () => props.field;
  const op = () => f().calc?.op ?? 'none';
  const chosen = () => f().calc?.fields ?? [];
  return (
    <>
      <Field label="Calculate">
        <select
          class="prepare-input"
          value={op()}
          onChange={(e) => props.patch({ calc: calcFor(e.currentTarget.value, chosen()) })}
        >
          <option value="none">Nothing</option>
          <For each={CALC_OPS}>{(c) => <option value={c.value}>{c.label} of</option>}</For>
        </select>
      </Field>
      <Show when={op() !== 'none'}>
        <div class="prepare-calc-fields" role="group" aria-label="Fields to calculate from">
          <Show
            when={props.others.length > 0}
            fallback={<p class="prepare-hint">No other text fields yet.</p>}
          >
            <For each={props.others}>
              {(name) => (
                <Check
                  label={name}
                  checked={chosen().includes(name)}
                  onChange={(v) =>
                    props.patch({
                      calc: calcFor(
                        op(),
                        v ? [...chosen(), name] : chosen().filter((n) => n !== name)
                      ),
                    })
                  }
                />
              )}
            </For>
          </Show>
        </div>
      </Show>
    </>
  );
}

function OptionsEditor(props: { field: DesignField; patch: Patch }) {
  const opts = () => props.field.options;
  const set = (next: FormOption[], key?: string) => props.patch({ options: next }, key);
  const update = (i: number, part: Partial<FormOption>, key: string) =>
    set(
      opts().map((o, j) => {
        if (j !== i) return { ...o };
        const next = { ...o, ...part };
        // An export value left equal to its label follows it.
        if (part.label !== undefined && o.export === o.label) next.export = part.label;
        return next;
      }),
      key
    );
  const move = (i: number, d: -1 | 1) => {
    const next = opts().map((o) => ({ ...o }));
    const j = i + d;
    if (j < 0 || j >= next.length) return;
    [next[i], next[j]] = [next[j]!, next[i]!];
    set(next);
  };
  return (
    <div class="prepare-options" role="group" aria-label="Options">
      <span class="prepare-label">Options (label, export value)</span>
      <For each={opts()}>
        {(o, i) => (
          <div class="prepare-option">
            <input
              class="prepare-input"
              aria-label={`Option ${i() + 1} label`}
              value={o.label}
              onInput={(e) => update(i(), { label: e.currentTarget.value }, `opt-label-${i()}`)}
            />
            <input
              class="prepare-input"
              aria-label={`Option ${i() + 1} export value`}
              value={o.export}
              onInput={(e) => update(i(), { export: e.currentTarget.value }, `opt-export-${i()}`)}
            />
            <button
              type="button"
              class="prepare-mini"
              aria-label="Move up"
              title="Move up"
              disabled={i() === 0}
              onClick={() => move(i(), -1)}
            >
              <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                <path
                  d="M3 7.5L6 4.5l3 3"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            </button>
            <button
              type="button"
              class="prepare-mini"
              aria-label="Move down"
              title="Move down"
              disabled={i() === opts().length - 1}
              onClick={() => move(i(), 1)}
            >
              <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                <path
                  d="M3 4.5L6 7.5l3-3"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            </button>
            <button
              type="button"
              class="prepare-mini"
              aria-label="Remove option"
              title="Remove"
              onClick={() => set(opts().filter((_, j) => j !== i()))}
            >
              <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                <path
                  d="M3 3l6 6M9 3l-6 6"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                />
              </svg>
            </button>
          </div>
        )}
      </For>
      <button
        type="button"
        class="secondary-btn"
        onClick={() => {
          const n = opts().length + 1;
          set([...opts().map((o) => ({ ...o })), { label: `Option ${n}`, export: `Option ${n}` }]);
        }}
      >
        Add option
      </button>
    </div>
  );
}

/** The page's fields in Tab order; drag a row (pointer events) to move it. */
function TabList(props: {
  fields: DesignField[];
  selected: string[];
  onSelect: (id: string) => void;
  onMove: (id: string, to: number) => void;
}) {
  let listEl!: HTMLOListElement;
  const [dragging, setDragging] = createSignal<{ id: string; to: number } | null>(null);
  const indexAt = (clientY: number) => {
    const rows = [...listEl.querySelectorAll<HTMLElement>('.prepare-tab-row')];
    let at = rows.length;
    for (let i = 0; i < rows.length; i += 1) {
      const r = rows[i]!.getBoundingClientRect();
      if (clientY < r.top + r.height / 2) {
        at = i;
        break;
      }
    }
    return at;
  };
  return (
    <ol class="prepare-tab-list" ref={listEl} aria-label="Fields in Tab order">
      <For each={props.fields}>
        {(f, i) => (
          <li
            class="prepare-tab-row"
            classList={{
              'is-selected': props.selected.includes(f.id),
              'is-target': dragging()?.to === i() && dragging()?.id !== f.id,
            }}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
              props.onSelect(f.id);
              setDragging({ id: f.id, to: i() });
            }}
            onPointerMove={(e) => {
              const d = dragging();
              if (d) setDragging({ ...d, to: indexAt(e.clientY) });
            }}
            onPointerUp={() => {
              const d = dragging();
              setDragging(null);
              if (!d) return;
              const from = props.fields.findIndex((x) => x.id === d.id);
              const to = d.to > from ? d.to - 1 : d.to;
              if (to !== from) props.onMove(d.id, to);
            }}
            onPointerCancel={() => setDragging(null)}
          >
            <span class="prepare-tab-num">{i() + 1}</span>
            <span class="prepare-tab-name">{f.name}</span>
            <span class="prepare-tab-kind">{kindLabel(f)}</span>
          </li>
        )}
      </For>
    </ol>
  );
}
