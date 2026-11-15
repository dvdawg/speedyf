/** KaTeX for comment bodies, as a TeX -> HTML function: the same renderer
 * and options the Notepad and titles use (`MathText`), loaded on first use.
 * Null until it has arrived; callers show the source meanwhile. */
import { createSignal } from 'solid-js';
import { loadKatex } from '../../components/MathText';

type Katex = { renderToString(src: string, options: object): string };

const [renderer, setRenderer] = createSignal<((tex: string, display: boolean) => string) | null>(
  null
);
let loading: Promise<void> | null = null;
const cache = new Map<string, string>();

/** Load KaTeX once; resolves (never rejects) when the attempt is over. */
export function loadMath(): Promise<void> {
  loading ??= Promise.all([loadKatex(), import('katex')])
    .then(([, mod]) => {
      const katex = ((mod as { default?: Katex }).default ?? mod) as Katex;
      setRenderer(() => (tex: string, display: boolean) => {
        const key = (display ? 'D' : 'I') + tex;
        let html = cache.get(key);
        if (html === undefined) {
          html = katex.renderToString(tex, {
            displayMode: display,
            throwOnError: false,
            trust: false,
            strict: 'ignore',
            output: 'html',
          });
          if (cache.size > 2000) cache.clear();
          cache.set(key, html);
        }
        return html;
      });
    })
    .catch(() => {
      // Math stays as its source; nothing else depends on it.
    });
  return loading;
}

/** The renderer, reactive: null until KaTeX has loaded. */
export function mathRenderer(): ((tex: string, display: boolean) => string) | null {
  return renderer();
}
