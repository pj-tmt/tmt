import { rendererBase } from './entry.js';
import { loadRuntime, wantsRuntime } from './page-runtime.js';
import type { StandaloneRequest } from './standalone.js';

/** What the export needs to know about the bundled runtime (#1693) for one page: the runtime the
 * app was loaded with when an ordinary page opts in, or why there is none. Any other page asks for
 * nothing, so its export stays as it was. The base is the renderer frame's own. */
export async function standaloneRequest(page: {
  source: string;
  pageType?: string;
}): Promise<StandaloneRequest | undefined> {
  if (page.pageType === 'shell' || !wantsRuntime(page.source)) return undefined;
  try {
    return { runtime: await loadRuntime(rendererBase()) };
  } catch {
    return { reason: 'runtime-unavailable' };
  }
}
