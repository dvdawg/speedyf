/** Active annotation tool + per-tool style memory. */
import { createStore, produce } from 'solid-js/store';

export type Tool = 'select' | 'highlight' | 'ink' | 'rect' | 'textbox' | 'note';

export interface ToolStyle {
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
  select: { color: '#1e88e5', opacity: 1, strokeWidth: 2, fontSizePt: 14 },
  highlight: { color: '#ffd54a', opacity: 0.45, strokeWidth: 0, fontSizePt: 14 },
  ink: { color: '#e53935', opacity: 1, strokeWidth: 2, fontSizePt: 14 },
  rect: { color: '#1e88e5', opacity: 1, strokeWidth: 2, fontSizePt: 14 },
  textbox: { color: '#d81b60', opacity: 1, strokeWidth: 1, fontSizePt: 14 },
  note: { color: '#fb8c00', opacity: 1, strokeWidth: 0, fontSizePt: 14 },
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
  setToolState(
    produce((s) => {
      Object.assign(s.styles[s.active], patch);
    })
  );
}
