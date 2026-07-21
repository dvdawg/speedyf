import type { JSX } from 'solid-js';

interface Props {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  children: JSX.Element;
  /** wider button with text content */
  wide?: boolean;
}

/** Toolbar button with tooltip, accessible name, disabled/active/focus states. */
export default function IconButton(props: Props) {
  return (
    <button
      type="button"
      class="icon-btn"
      classList={{ 'is-active': props.active, 'is-wide': props.wide }}
      title={props.label}
      aria-label={props.label}
      aria-pressed={props.active !== undefined ? props.active : undefined}
      disabled={props.disabled}
      onClick={() => props.onClick()}
    >
      {props.children}
    </button>
  );
}
