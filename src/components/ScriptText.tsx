/** Renders the script notation the engine reconstructs (see lib/text/scriptMarkup)
 * as real superscripts and subscripts. */
import { For } from 'solid-js';
import { parseScriptMarkup, plainScriptText } from '../lib/text/scriptMarkup';

export default function ScriptText(props: { value: string }) {
  const segments = () => parseScriptMarkup(props.value);
  return (
    <span class="script-text" title={plainScriptText(props.value)}>
      <For each={segments()}>
        {(segment) => (
          <>
            {segment.kind === 'super' && <sup>{segment.text}</sup>}
            {segment.kind === 'sub' && <sub>{segment.text}</sub>}
            {segment.kind === 'normal' && segment.text}
          </>
        )}
      </For>
    </span>
  );
}
