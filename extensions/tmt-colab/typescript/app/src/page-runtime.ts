import { isHostedOrigin } from './hosted-location.js';

/** The marker a page uses to ask for the bundled runtime (#1693): the first
 * `<meta name="tmt-colab-runtime" content="react tailwind">` in document order. */
const MARKER = 'tmt-colab-runtime';
const TOKENS = new Set(['react', 'tailwind']);
/** The runtime is one bundle, so any known token loads all of it; unknown tokens are ignored. */
export function wantsRuntime(source: string): boolean {
  if (!source.includes(MARKER)) return false;
  const meta = new DOMParser()
    .parseFromString(source, 'text/html')
    .querySelector(`meta[name="${MARKER}"]`);
  return (meta?.getAttribute('content') ?? '').split(/\s+/).some((token) => TOKENS.has(token));
}

/** What the frame bootstrap writes into its trusted head before the author's HTML. `style` is
 * reserved for the theme (empty until it ships); the bootstrap enforces the byte caps. */
export interface PageRuntime {
  readonly script: string;
  readonly style: string;
}

/** Hosted routes share the root build-owned asset; mounted routes stay mount-relative. */
export function runtimeUrl(baseUri: string): string {
  const base = new URL(baseUri);
  return new URL(
    isHostedOrigin(base.origin) ? '/assets/colab-runtime.js' : './assets/colab-runtime.js',
    base,
  ).href;
}

const loaded = new Map<string, Promise<PageRuntime>>();
/** Fetched once per origin path and shared by every opting-in page; a failure is not cached. */
export function loadRuntime(baseUri: string): Promise<PageRuntime> {
  const url = runtimeUrl(baseUri);
  let runtime = loaded.get(url);
  if (!runtime) {
    runtime = fetch(url, { credentials: 'same-origin' }).then(async (response) => {
      if (!response.ok) throw new Error('Page runtime unavailable');
      return Object.freeze({ script: await response.text(), style: '' });
    });
    runtime.catch(() => loaded.delete(url));
    loaded.set(url, runtime);
  }
  return runtime;
}
