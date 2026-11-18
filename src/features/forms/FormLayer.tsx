/** Interactive AcroForm widgets for one page.
 *
 * Every kind of field is filled where it is, on the page, rather than through
 * a list off to one side: a form asks its questions in place and answering it
 * anywhere else means holding the mapping between a field's name and the
 * sentence it belongs to in your head.
 *
 * Nothing here rewrites the page raster. Each widget is an overlay over its
 * own box. Until it is touched it is transparent and the raster shows the
 * file's value; once edited (or while focused) it covers the raster's now
 * stale appearance with paper and draws the value itself, at the field's
 * font size and alignment. The edit is recorded as a model edit, exactly like
 * an annotation, and the engine writes the real field at save time.
 */
import {
  createMemo,
  createResource,
  createSignal,
  For,
  Match,
  onCleanup,
  onMount,
  Show,
  Switch,
  useContext,
  type JSX,
} from 'solid-js';
import type { FormFieldInfo } from '../../types/engine';
import { formButtonKey } from '../document/documentStore';
import {
  choiceText,
  choiceValue,
  fieldLabel,
  formFields,
  isFillable,
  isOn,
  nextStop,
  registerWidget,
  textValue,
} from './formStore';
import { goToField } from './formNav';
import {
  COMB_FONT,
  FIELD_FONT,
  FIELD_PAD_PT,
  LINE_HEIGHT,
  combSpacing,
  fieldFontSize,
} from './fieldText';
import { TabContext } from '../../app/TabContext';
import { pagesInverted, settings } from '../../stores/settings';
import { invertInk } from '../../lib/rendering/invertColor';

interface Props {
  docId: number;
  src: number;
  pageHeightPt: number;
  zoom: number;
}

interface WidgetProps {
  f: FormFieldInfo;
  docId: number;
  zoom: number;
  pageHeightPt: number;
  /** Handles the keys every widget shares (Tab, Escape); true if it did. */
  onKey: (e: KeyboardEvent, f: FormFieldInfo) => boolean;
  /** Every field in the document, for radio groups. */
  all: () => FormFieldInfo[];
}

const ALIGN = ['left', 'center', 'right'] as const;

export default function FormLayer(props: Props) {
  const tab = useContext(TabContext)!;
  const [fields] = createResource(
    () => props.docId,
    (docId) => formFields(docId)
  );
  const all = () => fields() ?? [];
  const widgets = () => all().filter((f) => f.page === props.src && isFillable(f));

  const navigate = (from: FormFieldInfo, direction: 1 | -1) => {
    const stop = nextStop(all(), from, direction);
    if (stop) goToField(tab, all(), stop);
  };

  const onKey = (e: KeyboardEvent, f: FormFieldInfo): boolean => {
    if (e.key === 'Tab' && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      navigate(f, e.shiftKey ? -1 : 1);
      return true;
    }
    if (e.key === 'Escape') {
      // Leave the field, and nothing else: the window's Escape would go on
      // to close a panel or drop a tool.
      e.preventDefault();
      e.stopPropagation();
      (e.currentTarget as HTMLElement).blur();
      return true;
    }
    return false;
  };

  const widgetProps = (f: FormFieldInfo): WidgetProps => ({
    f,
    docId: props.docId,
    get zoom() {
      return props.zoom;
    },
    get pageHeightPt() {
      return props.pageHeightPt;
    },
    onKey,
    all,
  });

  return (
    <Show when={widgets().length > 0}>
      <div class="form-layer" classList={{ 'is-tinted': settings.highlightFields }}>
        <For each={widgets()}>
          {(f) => (
            <Switch>
              <Match when={f.kind === 'text'}>
                <TextWidget {...widgetProps(f)} onEnter={() => navigate(f, 1)} />
              </Match>
              <Match when={f.kind === 'combo'}>
                <ComboWidget {...widgetProps(f)} />
              </Match>
              <Match when={f.kind === 'list'}>
                <ListWidget {...widgetProps(f)} />
              </Match>
              <Match when={f.kind === 'checkbox' || f.kind === 'radio'}>
                <ButtonWidget {...widgetProps(f)} />
              </Match>
            </Switch>
          )}
        </For>
      </div>
    </Show>
  );
}

/** The widget's box in the unrotated page space the layer sits in. */
function boxStyle(p: WidgetProps): JSX.CSSProperties {
  return {
    left: `${p.f.rect.x * p.zoom}px`,
    top: `${(p.pageHeightPt - p.f.rect.y - p.f.rect.h) * p.zoom}px`,
    width: `${p.f.rect.w * p.zoom}px`,
    height: `${p.f.rect.h * p.zoom}px`,
  };
}

/** The field's text colour as ink on this page: inverted pages invert it the
 * way the engine inverted the raster around it. */
function inkOf(f: FormFieldInfo): string {
  return pagesInverted() ? invertInk(f.color) : f.color;
}

/** Make the element reachable by Tab and panel jumps for as long as it is
 * mounted. */
function useRegistration(p: WidgetProps, el: () => HTMLElement | undefined) {
  onMount(() => {
    const node = el();
    if (node) onCleanup(registerWidget(p.docId, p.f.page, p.f.index, node));
  });
}

function TextWidget(p: WidgetProps & { onEnter: () => void }) {
  const { documentStore } = useContext(TabContext)!;
  const doc = documentStore.state;
  // eslint-disable-next-line solid/reactivity -- fixed per widget: <For> keys widgets by field
  const f = p.f;
  let input: HTMLInputElement | HTMLTextAreaElement | undefined;
  useRegistration(p, () => input);
  const [focused, setFocused] = createSignal(false);
  // One undo step per visit to the field, however many keystrokes it took.
  let typingGroup = Symbol('form text edit');

  const value = () => textValue(doc, f);
  const covered = () => focused() || doc.formEdits[f.name] !== undefined;
  const sizePt = createMemo(() => fieldFontSize(value(), f));
  const comb = () => f.comb && f.maxLen > 0;

  const style = (): JSX.CSSProperties => {
    const base: JSX.CSSProperties = {
      ...boxStyle(p),
      '--form-ink': inkOf(f),
      'font-size': `${sizePt() * p.zoom}px`,
      'text-align': ALIGN[f.align] ?? 'left',
    };
    if (comb()) {
      const spacing = combSpacing(f, sizePt());
      base['letter-spacing'] = `${spacing.letterSpacing * p.zoom}px`;
      base['--form-pad'] = `${spacing.inset * p.zoom}px`;
      base['font-family'] = COMB_FONT;
      base['text-align'] = 'left';
    } else {
      base['--form-pad'] = `${FIELD_PAD_PT * p.zoom}px`;
      base['font-family'] = FIELD_FONT;
    }
    return base;
  };

  const handlers = {
    onFocus: () => {
      typingGroup = Symbol('form text edit');
      setFocused(true);
    },
    onBlur: () => setFocused(false),
    onInput: (e: InputEvent & { currentTarget: HTMLInputElement | HTMLTextAreaElement }) =>
      documentStore.apply(
        { type: 'setForm', field: f.name, value: e.currentTarget.value },
        typingGroup
      ),
    onKeyDown: (e: KeyboardEvent) => {
      if (p.onKey(e, f)) return;
      if (e.key === 'Enter' && !f.multiline && !e.isComposing) {
        e.preventDefault();
        p.onEnter();
      }
    },
  };

  return (
    <div
      class="form-field form-text"
      classList={{
        'is-covered': covered(),
        'form-cover': covered(),
        'is-required': f.required,
        'is-empty': value() === '',
        'is-comb': comb(),
        'is-multiline': f.multiline,
      }}
      data-align={comb() ? 'left' : (ALIGN[f.align] ?? 'left')}
      style={style()}
    >
      <Show
        when={f.multiline}
        fallback={
          <input
            ref={(el) => (input = el)}
            class="form-input"
            type={f.password ? 'password' : 'text'}
            value={value()}
            maxLength={f.maxLen > 0 ? f.maxLen : undefined}
            spellcheck={false}
            autocomplete="off"
            aria-label={fieldLabel(f)}
            aria-required={f.required}
            {...handlers}
          />
        }
      >
        <textarea
          ref={(el) => (input = el)}
          class="form-input"
          value={value()}
          maxLength={f.maxLen > 0 ? f.maxLen : undefined}
          spellcheck={false}
          aria-label={fieldLabel(f)}
          aria-required={f.required}
          style={{ 'line-height': String(LINE_HEIGHT) }}
          {...handlers}
        />
      </Show>
    </div>
  );
}

function ComboWidget(p: WidgetProps) {
  const { documentStore } = useContext(TabContext)!;
  const doc = documentStore.state;
  // eslint-disable-next-line solid/reactivity -- fixed per widget: <For> keys widgets by field
  const f = p.f;
  let control: HTMLElement | undefined;
  useRegistration(p, () => control);
  const [focused, setFocused] = createSignal(false);
  let typingGroup = Symbol('form choice edit');

  const choice = () => choiceValue(doc, f);
  const text = () => choiceText(f, choice());
  const covered = () => focused() || doc.formChoiceEdits[f.name] !== undefined;
  const sizePt = createMemo(() =>
    fieldFontSize(text(), { ...f, multiline: false, rect: { ...f.rect, w: f.rect.w - f.rect.h } })
  );

  const pick = (index: number) =>
    documentStore.apply({ type: 'setFormChoice', field: f.name, choice: { selected: [index] } });

  const type = (value: string) => {
    // Text that names an option is that option, so /V gets its export value.
    const index = f.options.findIndex((o) => o.label === value);
    documentStore.apply(
      {
        type: 'setFormChoice',
        field: f.name,
        choice: index >= 0 ? { selected: [index] } : { selected: [], text: value },
      },
      typingGroup
    );
  };

  const options = (
    <For each={f.options}>
      {(o, i) => (
        <option
          value={String(i())}
          selected={choice().text === undefined && choice().selected[0] === i()}
        >
          {o.label}
        </option>
      )}
    </For>
  );

  return (
    <div
      class="form-field form-combo"
      classList={{
        'is-covered': covered(),
        'form-cover': covered(),
        'is-required': f.required,
        'is-editable': f.editable,
      }}
      style={{
        ...boxStyle(p),
        '--form-ink': inkOf(f),
        '--form-pad': `${FIELD_PAD_PT * p.zoom}px`,
        'font-size': `${sizePt() * p.zoom}px`,
        'font-family': FIELD_FONT,
        'text-align': ALIGN[f.align] ?? 'left',
      }}
      onFocusIn={() => {
        typingGroup = Symbol('form choice edit');
        setFocused(true);
      }}
      onFocusOut={() => setFocused(false)}
    >
      <Show
        when={f.editable}
        fallback={
          <select
            ref={(el) => (control = el)}
            class="form-input form-select"
            aria-label={fieldLabel(f)}
            aria-required={f.required}
            onChange={(e) => pick(Number(e.currentTarget.value))}
            onKeyDown={(e) => p.onKey(e, f)}
          >
            <option value="" disabled selected={choice().selected.length === 0} hidden />
            {options}
          </select>
        }
      >
        <input
          ref={(el) => (control = el)}
          class="form-input"
          type="text"
          value={text()}
          spellcheck={false}
          autocomplete="off"
          role="combobox"
          aria-label={fieldLabel(f)}
          aria-required={f.required}
          aria-expanded={false}
          onInput={(e) => type(e.currentTarget.value)}
          onKeyDown={(e) => p.onKey(e, f)}
        />
        {/* The native menu, opened from the arrow end of the box. */}
        <select
          class="form-combo-picker"
          tabIndex={-1}
          aria-label={`${fieldLabel(f)} choices`}
          onChange={(e) => {
            pick(Number(e.currentTarget.value));
            control?.focus();
          }}
          style={{ width: `${Math.min(f.rect.h, f.rect.w / 2) * p.zoom}px` }}
        >
          <option value="" disabled selected hidden />
          {options}
        </select>
      </Show>
    </div>
  );
}

function ListWidget(p: WidgetProps) {
  const { documentStore } = useContext(TabContext)!;
  const doc = documentStore.state;
  // eslint-disable-next-line solid/reactivity -- fixed per widget: <For> keys widgets by field
  const f = p.f;
  let box: HTMLDivElement | undefined;
  useRegistration(p, () => box);
  const [focused, setFocused] = createSignal(false);
  const [active, setActive] = createSignal(f.selected[0] ?? 0);
  let anchor: number | null = null;

  const selected = () => choiceValue(doc, f).selected;
  const covered = () => focused() || doc.formChoiceEdits[f.name] !== undefined;
  const sizePt = () => (f.fontSize > 0 ? f.fontSize : 12);

  const set = (next: number[]) =>
    documentStore.apply({
      type: 'setFormChoice',
      field: f.name,
      choice: { selected: [...new Set(next)].sort((a, b) => a - b) },
    });

  const range = (a: number, b: number) =>
    Array.from({ length: Math.abs(a - b) + 1 }, (_, k) => Math.min(a, b) + k);

  const choose = (index: number, extend: boolean, toggle: boolean) => {
    setActive(index);
    if (!f.multiSelect) {
      set([index]);
      anchor = index;
      return;
    }
    if (toggle) {
      const now = selected();
      set(now.includes(index) ? now.filter((i) => i !== index) : [...now, index]);
      anchor = index;
    } else if (extend && anchor !== null) {
      set(range(anchor, index));
    } else {
      set([index]);
      anchor = index;
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (p.onKey(e, f)) return;
    const count = f.options.length;
    if (count === 0) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const next = Math.min(count - 1, Math.max(0, active() + (e.key === 'ArrowDown' ? 1 : -1)));
      choose(next, e.shiftKey, false);
      box
        ?.querySelector<HTMLElement>(`[data-option='${next}']`)
        ?.scrollIntoView({ block: 'nearest' });
    } else if (e.key === ' ' && f.multiSelect) {
      e.preventDefault();
      choose(active(), false, true);
    }
  };

  return (
    <div
      ref={box}
      class="form-field form-listbox"
      classList={{
        'is-covered': covered(),
        'form-cover': covered(),
        'is-required': f.required,
      }}
      role="listbox"
      tabIndex={0}
      aria-label={fieldLabel(f)}
      aria-required={f.required}
      aria-multiselectable={f.multiSelect}
      style={{
        ...boxStyle(p),
        '--form-ink': inkOf(f),
        '--form-pad': `${FIELD_PAD_PT * p.zoom}px`,
        'font-size': `${sizePt() * p.zoom}px`,
        'font-family': FIELD_FONT,
        'line-height': String(LINE_HEIGHT),
      }}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onKeyDown={onKeyDown}
    >
      <For each={f.options}>
        {(o, i) => (
          <div
            class="form-option"
            classList={{ 'is-selected': selected().includes(i()), 'is-active': active() === i() }}
            role="option"
            data-option={i()}
            aria-selected={selected().includes(i())}
            onMouseDown={(e) => {
              e.preventDefault();
              box?.focus();
              choose(i(), e.shiftKey, e.metaKey || e.ctrlKey);
            }}
          >
            {o.label}
          </div>
        )}
      </For>
    </div>
  );
}

function ButtonWidget(p: WidgetProps) {
  const tab = useContext(TabContext)!;
  const { documentStore } = tab;
  const doc = documentStore.state;
  // eslint-disable-next-line solid/reactivity -- fixed per widget: <For> keys widgets by field
  const f = p.f;
  let button: HTMLButtonElement | undefined;
  useRegistration(p, () => button);

  const checked = () => isOn(doc, f);
  /** The file draws this widget the other way now: cover its appearance. */
  const covered = () => {
    const edit = doc.formButtonEdits[formButtonKey(f.page, f.index)];
    return edit !== undefined && edit !== f.checked;
  };

  const group = () =>
    p
      .all()
      .filter((o) => o.kind === 'radio' && o.name === f.name && isFillable(o))
      .sort((a, b) => a.tabOrder - b.tabOrder);

  const setOn = (target: FormFieldInfo, next: boolean) => {
    documentStore.apply({
      type: 'setFormButton',
      page: target.page,
      index: target.index,
      checked: next,
    });
    // Selecting a radio clears the rest of its group. The engine enforces
    // that when it writes the field — the group's value lives on the shared
    // parent — but the overlay has to show it immediately, or two buttons
    // would appear selected until the file was saved and reopened.
    if (next && target.kind === 'radio') {
      for (const other of p.all()) {
        if (
          other.kind === 'radio' &&
          other.name === target.name &&
          !(other.page === target.page && other.index === target.index) &&
          isOn(doc, other)
        ) {
          documentStore.apply({
            type: 'setFormButton',
            page: other.page,
            index: other.index,
            checked: false,
          });
        }
      }
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (p.onKey(e, f)) return;
    if (f.kind !== 'radio') return;
    const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
    if (step === undefined) return;
    e.preventDefault();
    // Arrows move the selection round the group, as in any radio group.
    const members = group();
    const at = members.findIndex((m) => m.page === f.page && m.index === f.index);
    const next = members[(at + step + members.length) % members.length];
    if (!next) return;
    setOn(next, true);
    goToField(tab, p.all(), next);
  };

  return (
    <button
      ref={button}
      type="button"
      class="form-field form-widget"
      classList={{
        'is-checked': checked(),
        'is-radio': f.kind === 'radio',
        'is-required': f.required,
      }}
      role={f.kind === 'radio' ? 'radio' : 'checkbox'}
      aria-checked={checked()}
      aria-label={fieldLabel(f) || (f.kind === 'radio' ? 'Radio button' : 'Checkbox')}
      aria-required={f.required}
      style={{ ...boxStyle(p), '--form-ink': inkOf(f) }}
      onClick={() => setOn(f, !checked())}
      onKeyDown={onKeyDown}
    >
      <span
        class="form-check-face"
        classList={{
          'is-covered': covered(),
          'form-cover': covered(),
          'is-on': covered() && checked(),
        }}
        aria-hidden="true"
      />
    </button>
  );
}
