import { createSignal, Show } from 'solid-js';
import IconButton from './IconButton';
import {
  IconChevronDown,
  IconChevronUp,
  IconEdit,
  IconFitPage,
  IconFitWidth,
  IconForm,
  IconHighlight,
  IconImage,
  IconInk,
  IconNote,
  IconOpen,
  IconRect,
  IconRedo,
  IconRotate,
  IconSave,
  IconSaveAs,
  IconSearch,
  IconSelect,
  IconSidebar,
  IconTextBox,
  IconUndo,
  IconZoomIn,
  IconZoomOut,
} from './icons';
import { documentStore } from '../features/document/documentStore';
import { openFromDialog, saveDocument } from '../features/document/controller';
import { requestScrollToPage, rotateView, setViewport, viewport } from '../stores/viewportStore';
import { applyFit, refreshFit, setZoomAnchored, zoomStep } from '../features/viewer/zoomController';
import { activeStyle, toggleTool, toolState, updateActiveStyle } from '../features/annotations/toolStore';
import { addImageFromDialog } from '../features/editor/editorActions';

export default function Toolbar() {
  const doc = documentStore.state;
  const loaded = () => doc.loaded;
  const pageCount = () => doc.pages.length;
  const [pageInput, setPageInput] = createSignal<string | null>(null);
  const [zoomInput, setZoomInput] = createSignal<string | null>(null);

  const commitPageInput = (raw: string) => {
    setPageInput(null);
    const n = parseInt(raw, 10);
    if (Number.isFinite(n) && n >= 1 && n <= pageCount()) requestScrollToPage(n - 1);
  };

  const commitZoomInput = (raw: string) => {
    setZoomInput(null);
    const n = parseFloat(raw.replace('%', ''));
    if (Number.isFinite(n) && n > 0) setZoomAnchored(n / 100);
  };

  const showStroke = () => toolState.active === 'ink' || toolState.active === 'rect';
  const showFont = () => toolState.active === 'textbox';
  const showStyle = () => toolState.active !== 'select';

  return (
    <div class="toolbar" role="toolbar" aria-label="Main toolbar">
      <div class="tb-group">
        <IconButton
          label="Toggle sidebar"
          active={viewport.sidebarOpen}
          onClick={() => setViewport('sidebarOpen', !viewport.sidebarOpen)}
        >
          <IconSidebar />
        </IconButton>
      </div>

      <div class="tb-group">
        <IconButton label="Open PDF (⌘O)" onClick={() => void openFromDialog()}>
          <IconOpen />
        </IconButton>
        <IconButton
          label="Save (⌘S)"
          disabled={!loaded() || !doc.dirty || doc.saving}
          onClick={() => void saveDocument(false)}
        >
          <IconSave />
        </IconButton>
        <IconButton
          label="Save As… (⇧⌘S)"
          disabled={!loaded() || doc.saving}
          onClick={() => void saveDocument(true)}
        >
          <IconSaveAs />
        </IconButton>
      </div>

      <div class="tb-group">
        <IconButton
          label="Undo (⌘Z)"
          disabled={doc.historyDepth === 0}
          onClick={() => documentStore.undo()}
        >
          <IconUndo />
        </IconButton>
        <IconButton
          label="Redo (⇧⌘Z)"
          disabled={doc.redoDepth === 0}
          onClick={() => documentStore.redo()}
        >
          <IconRedo />
        </IconButton>
      </div>

      <div class="tb-group">
        <IconButton
          label="Previous page (Page Up)"
          disabled={!loaded() || viewport.currentPage <= 0}
          onClick={() => requestScrollToPage(viewport.currentPage - 1)}
        >
          <IconChevronUp />
        </IconButton>
        <IconButton
          label="Next page (Page Down)"
          disabled={!loaded() || viewport.currentPage >= pageCount() - 1}
          onClick={() => requestScrollToPage(viewport.currentPage + 1)}
        >
          <IconChevronDown />
        </IconButton>
        <label class="tb-field" title="Current page">
          <span class="sr-only">Current page</span>
          <input
            class="tb-input page-input"
            type="text"
            inputmode="numeric"
            disabled={!loaded()}
            value={pageInput() ?? (loaded() ? String(viewport.currentPage + 1) : '–')}
            onFocus={(e) => {
              setPageInput(e.currentTarget.value);
              e.currentTarget.select();
            }}
            onInput={(e) => setPageInput(e.currentTarget.value)}
            onBlur={(e) => commitPageInput(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur();
              if (e.key === 'Escape') {
                setPageInput(null);
                e.currentTarget.blur();
              }
            }}
            aria-label="Current page number"
          />
        </label>
        <span class="tb-total" aria-label="Total pages">
          / {loaded() ? pageCount() : '–'}
        </span>
      </div>

      <div class="tb-group">
        <IconButton label="Zoom out (⌘−)" disabled={!loaded()} onClick={() => zoomStep(-1)}>
          <IconZoomOut />
        </IconButton>
        <input
          class="tb-input zoom-input"
          type="text"
          disabled={!loaded()}
          value={zoomInput() ?? `${Math.round(viewport.zoom * 100)}%`}
          onFocus={(e) => {
            setZoomInput(e.currentTarget.value);
            e.currentTarget.select();
          }}
          onInput={(e) => setZoomInput(e.currentTarget.value)}
          onBlur={(e) => commitZoomInput(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur();
            if (e.key === 'Escape') {
              setZoomInput(null);
              e.currentTarget.blur();
            }
          }}
          aria-label="Zoom percentage"
        />
        <IconButton label="Zoom in (⌘+)" disabled={!loaded()} onClick={() => zoomStep(1)}>
          <IconZoomIn />
        </IconButton>
        <IconButton
          label="Fit page"
          disabled={!loaded()}
          active={viewport.fitMode === 'fit-page'}
          onClick={() => applyFit('fit-page')}
        >
          <IconFitPage />
        </IconButton>
        <IconButton
          label="Fit width"
          disabled={!loaded()}
          active={viewport.fitMode === 'fit-width'}
          onClick={() => applyFit('fit-width')}
        >
          <IconFitWidth />
        </IconButton>
        <IconButton
          label="Rotate view 90°"
          disabled={!loaded()}
          onClick={() => {
            rotateView();
            refreshFit();
          }}
        >
          <IconRotate />
        </IconButton>
      </div>

      <div class="tb-group" role="group" aria-label="Annotation tools">
        <IconButton
          label="Select (Esc)"
          disabled={!loaded()}
          active={toolState.active === 'select'}
          onClick={() => toggleTool('select')}
        >
          <IconSelect />
        </IconButton>
        <IconButton
          label="Highlight text"
          disabled={!loaded()}
          active={toolState.active === 'highlight'}
          onClick={() => toggleTool('highlight')}
        >
          <IconHighlight />
        </IconButton>
        <IconButton
          label="Freehand ink"
          disabled={!loaded()}
          active={toolState.active === 'ink'}
          onClick={() => toggleTool('ink')}
        >
          <IconInk />
        </IconButton>
        <IconButton
          label="Rectangle outline"
          disabled={!loaded()}
          active={toolState.active === 'rect'}
          onClick={() => toggleTool('rect')}
        >
          <IconRect />
        </IconButton>
        <IconButton
          label="Text box"
          disabled={!loaded()}
          active={toolState.active === 'textbox'}
          onClick={() => toggleTool('textbox')}
        >
          <IconTextBox />
        </IconButton>
        <IconButton
          label="Sticky note"
          disabled={!loaded()}
          active={toolState.active === 'note'}
          onClick={() => toggleTool('note')}
        >
          <IconNote />
        </IconButton>
        <Show when={showStyle()}>
          <div class="tool-options" role="group" aria-label="Tool style">
            <input
              type="color"
              class="tb-color"
              value={activeStyle().color}
              onInput={(e) => updateActiveStyle({ color: e.currentTarget.value })}
              title="Color"
              aria-label="Annotation color"
            />
            <label class="tb-slider" title="Opacity">
              <span class="sr-only">Opacity</span>
              <input
                type="range"
                min="0.1"
                max="1"
                step="0.05"
                value={activeStyle().opacity}
                onInput={(e) => updateActiveStyle({ opacity: parseFloat(e.currentTarget.value) })}
                aria-label="Annotation opacity"
              />
            </label>
            <Show when={showStroke()}>
              <label class="tb-slider" title="Stroke width">
                <span class="sr-only">Stroke width</span>
                <input
                  type="range"
                  min="0.5"
                  max="12"
                  step="0.5"
                  value={activeStyle().strokeWidth}
                  onInput={(e) =>
                    updateActiveStyle({ strokeWidth: parseFloat(e.currentTarget.value) })
                  }
                  aria-label="Stroke width"
                />
              </label>
            </Show>
            <Show when={showFont()}>
              <label class="tb-field" title="Font size (pt)">
                <span class="sr-only">Font size in points</span>
                <input
                  class="tb-input font-input"
                  type="number"
                  min="6"
                  max="72"
                  value={activeStyle().fontSizePt}
                  onInput={(e) => {
                    const v = parseFloat(e.currentTarget.value);
                    if (Number.isFinite(v)) updateActiveStyle({ fontSizePt: v });
                  }}
                  aria-label="Font size in points"
                />
              </label>
            </Show>
          </div>
        </Show>
      </div>

      <div class="tb-group tb-right">
        <IconButton
          label="Add image to page"
          disabled={!loaded() || !viewport.editMode}
          onClick={() => void addImageFromDialog()}
        >
          <IconImage />
        </IconButton>
        <IconButton
          label="Form fields"
          disabled={!loaded()}
          active={viewport.formPanelOpen}
          onClick={() => setViewport('formPanelOpen', !viewport.formPanelOpen)}
        >
          <IconForm />
        </IconButton>
        <IconButton
          label="Edit mode (page reorder, delete, rotate)"
          disabled={!loaded()}
          active={viewport.editMode}
          onClick={() => setViewport('editMode', !viewport.editMode)}
        >
          <IconEdit />
        </IconButton>
        <IconButton
          label="Search (⌘F)"
          disabled={!loaded()}
          active={viewport.searchOpen}
          onClick={() => setViewport('searchOpen', !viewport.searchOpen)}
        >
          <IconSearch />
        </IconButton>
      </div>
    </div>
  );
}
