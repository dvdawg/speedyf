/** Review mode's toolbar button: a quiet icon beside the drawers' toggles,
 * there only while the Markup extension is switched on and a document is
 * open - any PDF, in the library or not. */
import { Show } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconMarkup } from '../../components/icons';
import { formatShortcut } from '../../app/shortcutLabel';
import { activeTab } from '../../stores/tabsStore';
import { markupAvailable, reviewing, toggleReviewing } from './markupStore';

export function MarkupToggle() {
  const loaded = () => activeTab()?.documentStore.state.loaded ?? false;
  return (
    <Show when={markupAvailable() && loaded()}>
      <IconButton
        label={`${reviewing() ? 'Stop reviewing' : 'Review this paper'} (${formatShortcut('mod+shift+m')})`}
        active={reviewing()}
        onClick={() => toggleReviewing()}
      >
        <IconMarkup />
      </IconButton>
    </Show>
  );
}
