/** A signature's artwork, drawn to fill whatever box it is put in. Used on
 * the page, in the Sign menu and in the create dialog, so all three show
 * the same curves the saved file will hold. */
import { For, Match, Switch } from 'solid-js';
import type { SignatureArt } from '../../types/model';
import { strokeD } from './art';

export default function SignatureArtView(props: { art: SignatureArt; color: string }) {
  return (
    <svg
      class="signature-art"
      viewBox={`0 0 ${props.art.w} ${props.art.h}`}
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <Switch>
        <Match when={props.art.type === 'drawn' && props.art}>
          {(art) => (
            <For each={art().strokes}>
              {(stroke) => (
                <path
                  d={strokeD(stroke)}
                  fill="none"
                  stroke={props.color}
                  stroke-width={art().width}
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              )}
            </For>
          )}
        </Match>
        <Match when={props.art.type === 'typed' && props.art}>
          {(art) => <path d={art().d} fill={props.color} />}
        </Match>
        <Match when={props.art.type === 'image' && props.art}>
          {(art) => (
            <image
              href={`data:image/png;base64,${art().png}`}
              width={art().w}
              height={art().h}
              preserveAspectRatio="none"
            />
          )}
        </Match>
      </Switch>
    </svg>
  );
}
