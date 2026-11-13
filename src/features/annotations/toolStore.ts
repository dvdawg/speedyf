/** Active annotation tool + per-tool style memory. */
import { createStore, produce } from 'solid-js/store';
import type { TextFormat } from '../../types/model';
import { DEFAULT_TEXT_STYLE } from './textStyle';

export type Tool = 'select' | 'highlight' | 'ink' | 'rect' | 'textbox' | 'note' | 'bubble';

export interface ToolStyle extends TextFormat {
  color: string;
  opacity: number;
  strokeWidth: number;
  fontSizePt: number;
}

interface ToolState {
  active: Tool;
  styles: Record<Tool, ToolStyle>;
}

const defaults: Record<Tool, ToolStyle> = {
  select: { ...DEFAULT_TEXT_STYLE, color: '#1e88e5', opacity: 1, strokeWidth: 2 },
  highlight: { ...DEFAULT_TEXT_STYLE, color: '#ffd54a', opacity: 0.45, strokeWidth: 0 },
  ink: { ...DEFAULT_TEXT_STYLE, color: '#e53935', opacity: 1, strokeWidth: 2 },
  rect: { ...DEFAULT_TEXT_STYLE, color: '#1e88e5', opacity: 1, strokeWidth: 2 },
  textbox: { ...DEFAULT_TEXT_STYLE, color: '#1a1a1a', opacity: 1, strokeWidth: 1 },
  note: { ...DEFAULT_TEXT_STYLE, color: '#fb8c00', opacity: 1, strokeWidth: 0 },
  // Near-black, because a shaded bubble is answering a question rather than
  // marking up a page: it has to read as filled-in ink on a printout and
  // survive whatever a grader scans it through.
  bubble: { ...DEFAULT_TEXT_STYLE, color: '#1a1a1a', opacity: 1, strokeWidth: 0 },
};

const [toolState, setToolState] = createStore<ToolState>({
  active: 'select',
  styles: structuredClone(defaults),
});

export { toolState };

export function setActiveTool(tool: Tool) {
  setToolState('active', tool);
}

export function toggleTool(tool: Tool) {
  setToolState('active', toolState.active === tool ? 'select' : tool);
}

export function activeStyle(): ToolStyle {
  return toolState.styles[toolState.active];
}

export function updateActiveStyle(patch: Partial<ToolStyle>) {
  updateToolStyle(toolState.active, patch);
}

export function updateToolStyle(tool: Tool, patch: Partial<ToolStyle>) {
  setToolState(
    produce((s) => {
      Object.assign(s.styles[tool], patch);
    })
  );
}
