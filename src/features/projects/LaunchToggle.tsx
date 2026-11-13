/** Home | Projects. The one control both launch screens share.
 *
 * It writes the setting rather than a local signal because App picks the
 * screen from the setting: the "launch screen" is simply whichever one was
 * showing last, which is also what the user would expect to come back to.
 * See projects.md §5. */
import { For } from 'solid-js';
import { settings, updateSettings } from '../../stores/settings';
import type { LaunchScreen } from '../../stores/settings';

const OPTIONS: { id: LaunchScreen; label: string }[] = [
  { id: 'home', label: 'Home' },
  { id: 'projects', label: 'Projects' },
];

export default function LaunchToggle(props: { class?: string }) {
  return (
    <div
      class={`projects-launch-toggle${props.class ? ` ${props.class}` : ''}`}
      role="radiogroup"
      aria-label="Launch screen"
    >
      <For each={OPTIONS}>
        {(option) => (
          <button
            type="button"
            role="radio"
            aria-checked={settings.launchScreen === option.id}
            classList={{ 'is-active': settings.launchScreen === option.id }}
            onClick={() => updateSettings({ launchScreen: option.id })}
          >
            {option.label}
          </button>
        )}
      </For>
    </div>
  );
}
