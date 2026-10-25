/** The standalone copy of a page that opts in to the bundled runtime (#1693) or uses the design system
 * (#5057).
 *
 * `page.html` is the exact admitted source and never changes. For a page whose source asks for the
 * runtime or uses the design system, the export adds `page.standalone.html`: the same document the
 * renderer frame builds, with the runtime's and the design system's scripts and styles written into
 * its head, so the page looks and works when it is opened from disk with no Colab and no network.
 * Native export writes the same bytes (`rust/tmt-colab/src/export/standalone.rs`);
 * `vectors/export-v1.json` pins both. */
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

/** The two texts the page runtime adds: the bytes of `assets/colab-runtime.js` and
 * `assets/colab-runtime.css` of the build the app was loaded from. */
export interface StandaloneRuntime {
  readonly script: string;
  readonly style: string;
}
/** The two texts the design system adds (#5057): the bytes of `assets/colab-page-ui.js` and
 * `assets/colab-page-ui.css` of the same build, which are what the renderer frame carries. */
export interface StandaloneDesign {
  readonly script: string;
  readonly style: string;
}
/** Why a page that needs a standalone copy got none. */
export type StandaloneReason =
  | 'runtime-unavailable'
  | 'runtime-unusable'
  | 'design-unavailable'
  | 'design-unusable';
/** What became of the copy; absent for a page that needs none. */
export interface StandaloneOutcome {
  readonly included: boolean;
  readonly reason?: StandaloneReason;
}
/** What the caller found for a page that needs a copy: the runtime and design texts the page asks for
 * that it loaded, or what it could not load. */
export type StandaloneRequest =
  | { readonly runtime?: StandaloneRuntime; readonly design?: StandaloneDesign }
  | { readonly reason: 'runtime-unavailable' | 'design-unavailable' };

/** A page uses the design system (#5057) when its source contains `tmt-ui-`, `<tmt-icon` or
 * `tmt-starter`, ignoring ASCII case. A plain text test, with no parsing: it is the test the renderer
 * frame makes to paint the page, and `vectors/design-marker-v1.json` pins this, the frame's and the
 * native export's. The `i` flag without `u` folds only ASCII letters that are in these names. */
export function usesDesign(source: string): boolean {
  return /tmt-ui-|<tmt-icon|tmt-starter/i.test(source);
}

/** A text that could end its own element or open a comment would escape the head block. The build
 * already refuses both; this is the guard. */
function escapes(text: string, closing: RegExp): boolean {
  return text.includes('<!--') || closing.test(text);
}

/** The copy's document, or why a text is not safe to write into HTML. The head holds, in order: the
 * design style and the runtime's theme (the frame carries them in that order), the theme switch, then
 * the design script and the runtime script. A page that needs only the runtime gets the same bytes as
 * before the design system. */
export function standaloneDocument(
  source: string,
  parts: { readonly runtime?: StandaloneRuntime; readonly design?: StandaloneDesign },
): string | { readonly reason: 'runtime-unusable' | 'design-unusable' } {
  const { runtime, design } = parts;
  if (runtime && (escapes(runtime.script, /<\/script/i) || escapes(runtime.style, /<\/style/i)))
    return { reason: 'runtime-unusable' };
  if (design && (escapes(design.script, /<\/script/i) || escapes(design.style, /<\/style/i)))
    return { reason: 'design-unusable' };
  return `<!doctype html><html${design ? ' data-colab-design' : ''}><head><meta http-equiv="Content-Security-Policy" content="${META_POLICY}"><meta charset="utf-8">${design ? `<style>${design.style}</style>` : ''}${runtime ? `<style type="text/tailwindcss">${runtime.style}</style>` : ''}<script>${THEME_SWITCH}</script>${design ? `<script>${design.script}</script>` : ''}${runtime ? `<script>${runtime.script}</script>` : ''}</head><body>${source}</body></html>`;
}

/** The copy of `source` (a page that needs one) and its outcome. */
export function deriveStandalone(
  source: string,
  request: StandaloneRequest,
): { outcome: StandaloneOutcome; document?: string } {
  if ('reason' in request) return { outcome: { included: false, reason: request.reason } };
  const document = standaloneDocument(source, request);
  return typeof document === 'string'
    ? { outcome: { included: true }, document }
    : { outcome: { included: false, reason: document.reason } };
}
