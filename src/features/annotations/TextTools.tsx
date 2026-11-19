import { Show } from 'solid-js';
import IconButton from '../../components/IconButton';
import OverflowRow, { type ToolbarGroup } from '../../components/OverflowRow';
import ToolbarField from '../../components/ToolbarField';
import { activeTab } from '../../stores/tabsStore';
import type { TextFontFamily } from '../../types/model';
import { toolState, updateToolStyle, type ToolStyle } from './toolStore';
import { fitTextBox, textStyleOf } from './textStyle';

/** A second toolbar row stays available for creating and editing text boxes.
 * It never wraps: what does not fit folds into its own overflow menu. Size
 * and bold/italic/underline stay longest (set on nearly every box), then
 * colour, then the font (wide, and set once), then opacity; the caption and
 * the hint simply leave first. */
export default function TextTools() {
  const editing = () => {
    if (toolState.active !== 'select' && toolState.active !== 'fill') return;
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

  const groups: ToolbarGroup[] = [
    {
      id: 'label',
      label: 'Text',
      priority: 2,
      drops: true,
      render: () => <span class="text-tool-label">Text</span>,
    },
    {
      id: 'font',
      label: 'Font family',
      priority: 4,
      render: () => (
        <ToolbarField label="Font">
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
        </ToolbarField>
      ),
    },
    {
      id: 'size',
      label: 'Size',
      priority: 7,
      render: () => (
        <ToolbarField label="Size">
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
        </ToolbarField>
      ),
    },
    {
      id: 'format',
      label: 'Style',
      priority: 6,
      render: () => (
        <>
          <IconButton
            label="Bold"
            active={style().bold}
            onClick={() => update({ bold: !style().bold })}
          >
            <strong class="text-format-glyph">B</strong>
          </IconButton>
          <IconButton
            label="Italic"
            active={style().italic}
            onClick={() => update({ italic: !style().italic })}
          >
            <em class="text-format-glyph">I</em>
          </IconButton>
          <IconButton
            label="Underline"
            active={style().underline}
            onClick={() => update({ underline: !style().underline })}
          >
            <u class="text-format-glyph">U</u>
          </IconButton>
        </>
      ),
    },
    {
      id: 'color',
      label: 'Color',
      priority: 5,
      render: () => (
        <ToolbarField label="Color">
          <input
            type="color"
            class="tb-color"
            aria-label="Text color"
            title="Text color"
            value={style().color}
            onInput={(e) => update({ color: e.currentTarget.value })}
          />
        </ToolbarField>
      ),
    },
    {
      id: 'opacity',
      label: 'Opacity',
      priority: 3,
      render: () => (
        <ToolbarField label="Opacity">
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
        </ToolbarField>
      ),
    },
    {
      id: 'hint',
      label: 'Hint',
      priority: 1,
      drops: true,
      render: () => (
        <span class="text-tool-hint">
          {editing() ? 'Applies to this text box' : 'Click the page to type'}
        </span>
      ),
    },
  ];

  return (
    <Show when={visible()}>
      <div
        class="text-tool-options"
        role="group"
        aria-label="Text style"
        onPointerDown={(e) => {
          // Formatting buttons should leave the caret (and selection) in
          // place; the overflow menu's rows reach here through its portal.
          if ((e.target as HTMLElement).closest('button')) e.preventDefault();
        }}
      >
        <OverflowRow
          groups={groups}
          class="options-row"
          menuClass="text-tool-menu"
          moreLabel="More text options"
        />
      </div>
    </Show>
  );
}
