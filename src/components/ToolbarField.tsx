/** A field (select, number, colour well, slider) in a toolbar row. In the
 * row it is the bare control; folded into the overflow menu it becomes a
 * labelled menu row with the same control at its end, still usable there. */
import { Show, useContext, type JSX } from 'solid-js';
import { ToolbarMenuContext } from './toolbarMenu';

export default function ToolbarField(props: { label: string; children: JSX.Element }) {
  const menu = useContext(ToolbarMenuContext);
  return (
    <Show when={menu} fallback={props.children}>
      <div class="tb-menu-field">
        <span class="tb-menu-name" aria-hidden="true">
          {props.label}
        </span>
        {props.children}
      </div>
    </Show>
  );
}
