import { Show, useContext, type JSX } from 'solid-js';
import { ToolbarMenuContext, type ToolbarMenu } from './toolbarMenu';
import { splitShortcut } from './toolbarOverflow';

interface Props {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  children: JSX.Element;
  /** wider button with text content */
  wide?: boolean;
  /** Opens a menu of its own: in the toolbar's overflow menu it expands in
   * place instead of closing the overflow. */
  submenu?: boolean;
}

/** Toolbar button with tooltip, accessible name, disabled/active/focus states.
 * Inside the toolbar's overflow menu the same control is a menu row: its
 * icon, its name, and its shortcut set apart. */
export default function IconButton(props: Props) {
  const menu = useContext(ToolbarMenuContext);
  return (
    <Show when={menu} fallback={<ToolbarButton {...props} />}>
      {(m) => <MenuRow {...props} menu={m()} />}
    </Show>
  );
}

function ToolbarButton(props: Props) {
  return (
    <button
      type="button"
      class="icon-btn"
      classList={{ 'is-active': props.active, 'is-wide': props.wide }}
      title={props.label}
      aria-label={props.label}
      aria-pressed={props.active !== undefined ? props.active : undefined}
      aria-haspopup={props.submenu ? 'menu' : undefined}
      disabled={props.disabled}
      onClick={() => props.onClick()}
    >
      {props.children}
    </button>
  );
}

function MenuRow(props: Props & { menu: ToolbarMenu }) {
  const parts = () => splitShortcut(props.label);
  return (
    <button
      type="button"
      class="tb-menu-item"
      classList={{ 'is-active': props.active }}
      role={props.active !== undefined ? 'menuitemcheckbox' : 'menuitem'}
      aria-label={props.label}
      aria-checked={props.active !== undefined ? props.active : undefined}
      aria-haspopup={props.submenu ? 'menu' : undefined}
      disabled={props.disabled}
      tabIndex={-1}
      onClick={() => {
        props.onClick();
        if (!props.submenu) props.menu.close();
      }}
    >
      <span class="tb-menu-icon" aria-hidden="true">
        {props.children}
      </span>
      <span class="tb-menu-name">{parts().name}</span>
      <Show when={parts().key}>{(key) => <span class="tb-menu-key">{key()}</span>}</Show>
    </button>
  );
}
