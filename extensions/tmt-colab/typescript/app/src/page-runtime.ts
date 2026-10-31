import { browserElements, mayUseBrowserElements } from '@tmt/browser-ui/elements-manifest';
import { isHostedOrigin } from './hosted-location.js';

/** The marker a page uses to ask for the bundled runtime (#1693): the first
 * `<meta name="tmt-colab-runtime" content="react tailwind">` in document order. */
const MARKER = 'tmt-colab-runtime';
const TOKENS = new Set(['react', 'tailwind']);
/** A registered widget needs the same bundle without an author script or meta marker. Known meta tokens still opt in; unknown tokens are ignored. */
export function wantsRuntime(source: string): boolean {
  if (!source.includes(MARKER) && !mayUseBrowserElements(source)) return false;
  const document = new DOMParser().parseFromString(source, 'text/html');
  if (
    [...document.querySelectorAll(browserElements.map(({ tag }) => tag).join(','))].some(
      (node) => node instanceof HTMLElement,
    )
  )
    return true;
  const meta = document.querySelector(`meta[name="${MARKER}"]`);
  return (meta?.getAttribute('content') ?? '')
    .split(/[ \t\n\f\r]+/)
    .some((token) => TOKENS.has(token));
}

/** What the frame bootstrap writes into its trusted head before the author's HTML: the theme
 * as the source of a `<style type="text/tailwindcss">` block, then the runtime script. The
 * bootstrap enforces the byte caps. */
export interface PageRuntime {
  readonly script: string;
  readonly style: string;
}

/** Hosted routes share the root build-owned asset; mounted routes stay mount-relative. */
export function runtimeUrl(baseUri: string, name = 'colab-runtime.js'): string {
  const base = new URL(baseUri);
  return new URL(isHostedOrigin(base.origin) ? `/assets/${name}` : `./assets/${name}`, base).href;
}

const loaded = new Map<string, Promise<PageRuntime>>();
const read = async (url: string): Promise<string> => {
  const response = await fetch(url, { credentials: 'same-origin' });
  if (!response.ok) throw new Error('Page runtime unavailable');
  return response.text();
};
/** Fetched once per origin path and shared by every opting-in page; a failure is not cached. */
export function loadRuntime(baseUri: string): Promise<PageRuntime> {
  const key = runtimeUrl(baseUri);
  let runtime = loaded.get(key);
  if (!runtime) {
    runtime = Promise.all([read(key), read(runtimeUrl(baseUri, 'colab-runtime.css'))]).then(
      ([script, style]) => Object.freeze({ script, style }),
    );
    runtime.catch(() => loaded.delete(key));
    loaded.set(key, runtime);
  }
  return runtime;
}

/** What a standalone export of a page that uses the design system carries (#5057): the same CSS and
 * `tmt-icon` script the renderer frame holds, built into the app as `colab-page-ui.css` and
 * `colab-page-ui.js`. */
export interface PageDesign {
  readonly script: string;
  readonly style: string;
}
const designs = new Map<string, Promise<PageDesign>>();
/** Fetched once per origin path; a failure is not cached. Only an export asks for it. */
export function loadDesign(baseUri: string): Promise<PageDesign> {
  const key = runtimeUrl(baseUri, 'colab-page-ui.js');
  let design = designs.get(key);
  if (!design) {
    design = Promise.all([read(key), read(runtimeUrl(baseUri, 'colab-page-ui.css'))]).then(
      ([script, style]) => Object.freeze({ script, style }),
    );
    design.catch(() => designs.delete(key));
    designs.set(key, design);
  }
  return design;
}
