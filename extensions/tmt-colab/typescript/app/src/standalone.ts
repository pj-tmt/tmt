/** The standalone copy of a page that opts in to the bundled runtime (#1693).
 *
 * `page.html` is the exact admitted source and never changes. For a page whose source asks for the
 * runtime, the export adds `page.standalone.html`: the same document the renderer frame builds,
 * with the runtime's script and theme written into its head, so the page works when it is opened
 * from disk with no Colab and no network. Native export writes the same bytes
 * (`rust/tmt-colab/src/export/standalone.rs`); `vectors/export-v1.json` pins both. */
import type { ExportFile } from './export.js';

/** The published name of the derived file, and the file the manifest says it is derived from. */
export const STANDALONE_FILE: ExportFile = 'page.standalone.html';
export const DERIVED_FROM: ExportFile = 'page.html';

/** The renderer frame's policy without its `sandbox` directive, the one a `<meta>` cannot carry:
 * the copy gains no network when it is opened from disk, but it is not sandboxed. The native
 * export holds the same text. */
export const META_POLICY =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; font-src 'none'; media-src 'none'; worker-src 'none'; manifest-src 'none'";

/** The theme follows `data-theme` only, so the copy sets it from the system colour scheme and
 * follows changes, the way Colab's own pages follow the app theme. */
export const THEME_SWITCH =
  "(()=>{const q=matchMedia('(prefers-color-scheme: dark)'),r=document.documentElement,s=()=>{r.dataset.theme=q.matches?'dark':'light'};s();q.addEventListener('change',s)})()";

/** The two texts the copy carries: the bytes of `assets/colab-runtime.js` and
 * `assets/colab-runtime.css` of the build the app was loaded from. */
export interface StandaloneRuntime {
  readonly script: string;
  readonly style: string;
}
/** Why a page that asks for the runtime got no standalone copy. */
export type StandaloneReason = 'runtime-unavailable' | 'runtime-unusable';
/** What became of the copy; absent for a page that does not opt in. */
export interface StandaloneOutcome {
  readonly included: boolean;
  readonly reason?: StandaloneReason;
}
/** What the caller found for a page that opts in: the runtime it loaded, or that it could not. */
export type StandaloneRequest =
  | { readonly runtime: StandaloneRuntime }
  | { readonly reason: 'runtime-unavailable' };

/** A runtime text that could end its own element or open a comment would escape the head block.
 * The build already refuses both; this is the guard. */
function escapes(text: string, closing: RegExp): boolean {
  return text.includes('<!--') || closing.test(text);
}

/** The copy's document, or `undefined` when a runtime text is not safe to write into HTML. */
export function standaloneDocument(source: string, runtime: StandaloneRuntime): string | undefined {
  if (escapes(runtime.script, /<\/script/i) || escapes(runtime.style, /<\/style/i)) {
    return undefined;
  }
  return `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${META_POLICY}"><meta charset="utf-8"><style type="text/tailwindcss">${runtime.style}</style><script>${THEME_SWITCH}</script><script>${runtime.script}</script></head><body>${source}</body></html>`;
}

/** The copy of `source` (a page that opts in) and its outcome. */
export function deriveStandalone(
  source: string,
  request: StandaloneRequest,
): { outcome: StandaloneOutcome; document?: string } {
  if (!('runtime' in request)) return { outcome: { included: false, reason: request.reason } };
  const document = standaloneDocument(source, request.runtime);
  return document === undefined
    ? { outcome: { included: false, reason: 'runtime-unusable' } }
    : { outcome: { included: true }, document };
}
