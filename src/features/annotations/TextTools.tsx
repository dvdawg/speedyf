import { Show } from 'solid-js';
import { activeTab } from '../../stores/tabsStore';
import type { TextFontFamily } from '../../types/model';
import { toolState, updateToolStyle, type ToolStyle } from './toolStore';
import { fitTextBox, textStyleOf } from './textStyle';

/** A second toolbar row stays available for creating and editing text boxes. */
export default function TextTools() {
  const editing = () => {
    if (toolState.active !== 'select') return;
    const tab = activeTab();
    const state = tab?.documentStore.state;
    const sel = tab?.viewport.state.editingText;
    if (
      !state ||
      !sel ||
      state.selected?.annotId !== sel.annotId ||
      state.selected.pageId !== sel.pageId
    )
      return;
    return state.annotations[sel.pageId]?.find((a) => a.id === sel.annotId && a.kind === 'textbox');
  };
  const visible = () =>
    activeTab()?.documentStore.state.loaded && (toolState.active === 'textbox' || editing());
  const style = () => {
    const a = editing();
    return a ? { ...textStyleOf(a), color: a.color, opacity: a.opacity } : toolState.styles.textbox;
  };
  const update = (patch: Partial<ToolStyle>) => {
    updateToolStyle('textbox', patch);
    const a = editing();
    const store = activeTab()?.documentStore;
    const page = store?.state.pages.find((p) => p.id === a?.pageId);
    if (!a || !store || !page) return;
    store.apply({
      type: 'patchAnnot',
      pageId: a.pageId,
      id: a.id,
      patch: { ...patch, rect: fitTextBox({ ...a, ...patch }, page) },
    });
  };

  return (
    <Show when={visible()}>
      <div
        class="text-tool-options"
        role="group"
        aria-label="Text style"
        onPointerDown={(e) => {
          // Formatting buttons should leave the caret (and selection) in place.
          if ((e.target as HTMLElement).closest('button')) e.preventDefault();
        }}
      >
        <span class="text-tool-label">Text</span>
        <select
          class="tb-input font-family-input"
          aria-label="Text font"
          title="Font"
          value={style().fontFamily}
          onChange={(e) => update({ fontFamily: e.currentTarget.value as TextFontFamily })}
        >
          <option value="Helvetica">Helvetica</option>
          <option value="Times-Roman">Times New Roman</option>
          <option value="Courier">Courier</option>
        </select>
        <label class="tb-field" title="Font size (pt)">
          <input
            class="tb-input font-input"
            type="number"
            min="6"
            max="144"
            step="1"
            value={style().fontSizePt}
            aria-label="Font size in points"
            onInput={(e) => {
              const size = e.currentTarget.valueAsNumber;
              if (Number.isFinite(size) && size >= 6 && size <= 144) update({ fontSizePt: size });
            }}
            onBlur={(e) => {
              e.currentTarget.value = String(style().fontSizePt);
            }}
          />
          <span class="text-tool-unit">pt</span>
        </label>
        <div class="text-format-buttons">
          <button
            type="button"
            class="icon-btn"
            classList={{ 'is-active': style().bold }}
            aria-label="Bold"
            title="Bold"
            aria-pressed={style().bold}
            onClick={() => update({ bold: !style().bold })}
          >
            <strong>B</strong>
          </button>
          <button
            type="button"
            class="icon-btn"
            classList={{ 'is-active': style().italic }}
            aria-label="Italic"
            title="Italic"
            aria-pressed={style().italic}
            onClick={() => update({ italic: !style().italic })}
          >
            <em>I</em>
          </button>
          <button
            type="button"
            class="icon-btn"
            classList={{ 'is-active': style().underline }}
            aria-label="Underline"
            title="Underline"
            aria-pressed={style().underline}
            onClick={() => update({ underline: !style().underline })}
          >
            <u>U</u>
          </button>
        </div>
        <input
          type="color"
          class="tb-color"
          aria-label="Text color"
          title="Text color"
          value={style().color}
          onInput={(e) => update({ color: e.currentTarget.value })}
        />
        <label class="tb-slider" title="Text opacity">
          <span class="sr-only">Text opacity</span>
          <input
            type="range"
            min="0.1"
            max="1"
            step="0.05"
            aria-label="Text opacity"
            value={style().opacity}
            onInput={(e) => update({ opacity: e.currentTarget.valueAsNumber })}
          />
        </label>
        <span class="text-tool-hint">
          {editing() ? 'Applies to this text box' : 'Click the page to type'}
        </span>
      </div>
    </Show>
  );
}
