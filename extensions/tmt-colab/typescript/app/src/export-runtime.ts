import { rendererBase } from './entry.js';
import { loadDesign, loadRuntime, wantsRuntime } from './page-runtime.js';
import { usesDesign, type StandaloneRequest } from './standalone.js';

/** What the export needs to know about the bundled runtime (#1693) and the design system (#5057) for
 * one page: what the app was loaded with that the page asks for (an ordinary page that opts in to the
 * runtime, a page that uses the design system, or both), or why there is none. Any other page asks for
 * nothing, so its export stays as it was. The base is the renderer frame's own. */
export async function standaloneRequest(page: {
  source: string;
  pageType?: string;
}): Promise<StandaloneRequest | undefined> {
  if (page.pageType === 'shell') return undefined;
  const wantsRuntimeBundle = wantsRuntime(page.source);
  const wantsDesign = usesDesign(page.source);
  if (!wantsRuntimeBundle && !wantsDesign) return undefined;
  const base = rendererBase();
  const [runtime, design] = await Promise.allSettled([
    wantsRuntimeBundle ? loadRuntime(base) : undefined,
    wantsDesign ? loadDesign(base) : undefined,
  ]);
  if (runtime.status === 'rejected') return { reason: 'runtime-unavailable' };
  if (design.status === 'rejected') return { reason: 'design-unavailable' };
  return {
    ...(runtime.value === undefined ? {} : { runtime: runtime.value }),
    ...(design.value === undefined ? {} : { design: design.value }),
  };
}
