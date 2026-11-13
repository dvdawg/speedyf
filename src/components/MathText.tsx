/** Text with TeX math in it (`$x^2$`, `\(a\)`, `\[b\]`), math typeset by
 * KaTeX and everything else as plain, escaped text.
 *
 * KaTeX and its stylesheet load on the first piece of math the app shows, not
 * at startup; both are bundled by Vite (the fonts resolve from 'self', as the
 * CSP requires). Until they arrive each math run shows its raw source. */
import { createSignal, For, Show } from 'solid-js';
import { splitMath, type MathSegment } from './mathSplit';

export { splitMath, hasMath, plainMath, type MathSegment } from './mathSplit';

type Katex = { renderToString(src: string, options: object): string };

let katex: Katex | null = null;
let loading: Promise<void> | null = null;
const [ready, setReady] = createSignal(false);

/** Load KaTeX once. Resolves (never rejects) when the attempt is over. */
export function loadKatex(): Promise<void> {
  loading ??= Promise.all([import('katex'), import('katex/dist/katex.min.css')])
    .then(([mod]) => {
      katex = (mod.default ?? mod) as Katex;
      setReady(true);
    })
    .catch(() => {
      // Math stays as its source text; nothing else depends on it.
    });
  return loading;
}

const RENDER_LIMIT = 2000;
const rendered = new Map<string, string>();

function renderMath(src: string, display: boolean): string {
  const key = (display ? 'D' : 'I') + src;
  let html = rendered.get(key);
  if (html === undefined) {
    // `\[...\]` keeps its display-style layout but stays in the line: a title
    // or abstract is no place for a centered block.
    html = katex!.renderToString(display ? `\\displaystyle ${src}` : src, {
      throwOnError: false,
      trust: false,
      strict: 'ignore',
      output: 'html',
    });
    if (rendered.size >= RENDER_LIMIT) rendered.clear();
    rendered.set(key, html);
  }
  return html;
}

/** One whole formula, typeset in KaTeX's display mode when `display`: a real
 * centred block, where `MathText` keeps display math in the line. */
function renderFormula(src: string, display: boolean): string {
  const key = (display ? 'B' : 'F') + src;
  let html = rendered.get(key);
  if (html === undefined) {
    html = katex!.renderToString(src, {
      displayMode: display,
      throwOnError: false,
      trust: false,
      strict: 'ignore',
      output: 'html',
    });
    if (rendered.size >= RENDER_LIMIT) rendered.clear();
    rendered.set(key, html);
  }
  return html;
}

export function MathFormula(props: { tex: string; display: boolean; class?: string }) {
  if (!ready()) void loadKatex();
  return (
    <Show when={ready()} fallback={<code class="math-raw">{props.tex}</code>}>
      {/* KaTeX output from `trust: false`, as in MathRun. */}
      {/* eslint-disable-next-line solid/no-innerhtml */}
      <div class={props.class} innerHTML={renderFormula(props.tex, props.display)} />
    </Show>
  );
}

function MathRun(props: { segment: Extract<MathSegment, { kind: 'math' }> }) {
  if (!ready()) void loadKatex();
  return (
    <Show when={ready()} fallback={<span class="math-raw">{props.segment.raw}</span>}>
      {/* KaTeX output from `trust: false`: it escapes the source itself and
          emits no links, scripts or raw HTML. Plain text never comes here. */}
      {/* eslint-disable-next-line solid/no-innerhtml */}
      <span class="math-run" innerHTML={renderMath(props.segment.src, props.segment.display)} />
    </Show>
  );
}

export default function MathText(props: { text: string; class?: string }) {
  const segments = () => splitMath(props.text);
  const body = () => (
    <For each={segments()}>
      {(segment) => (segment.kind === 'text' ? segment.text : <MathRun segment={segment} />)}
    </For>
  );
  return (
    <Show when={props.class} fallback={body()}>
      <span class={props.class}>{body()}</span>
    </Show>
  );
}
