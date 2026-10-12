/** A page whose serve answers usage and compact as the spec scripts. Never imported by production. */
import { createRoot, type Root } from 'react-dom/client';
import { RouterProvider } from '@tanstack/react-router';
import { createAppRouter } from '../src/router.js';
import type { CompactReport, PageUsage } from '../src/page-usage.js';
import { SaveRefused } from '../src/save.js';
import { localTransport, type PageBinding } from '../src/transport.js';

export interface PageSizeScript {
  usage: PageUsage;
  /** What the next compact answers; `null` makes it fail as a lost reply would. */
  report: CompactReport | null;
  /** The next save is refused for a full page. */
  refuseSave: boolean;
  /** The binding has no usage or compact, as a reader's does. */
  reader: boolean;
  calls: { usage: number; compact: number; edit: number; exported: number };
}
declare global {
  interface Window {
    __pageSize?: PageSizeScript;
  }
}
let root: Root | undefined;

export function usageOf(extra: Partial<PageUsage> = {}): PageUsage {
  return {
    updates: 124,
    updateLimit: 200,
    tailBytes: 1_258_291,
    tailBytesLimit: 4_194_304,
    checkpointBytes: 4000,
    otherDeviceUpdates: 0,
    stateBytes: 1_262_291,
    gzipLimit: 5_000_000,
    percent: 62,
    limiting: 'changes',
    ...extra,
  };
}
export function reportOf(
  extra: Partial<Omit<CompactReport, 'before' | 'after'>> & {
    before?: Partial<PageUsage>;
    after?: Partial<PageUsage>;
  } = {},
): CompactReport {
  const { before, after, ...rest } = extra;
  return {
    pageId: '10000000-0000-4000-8000-000000000001',
    compacted: true,
    by: '30000000-0000-4000-8000-000000000001',
    through: 124,
    mergedObjects: 124,
    sourceSha256: 'a'.repeat(64),
    before: usageOf(before),
    after: usageOf({ updates: 1, tailBytes: 62, percent: 0, ...after }),
    ...rest,
  };
}

export function mountPageSize(first: Partial<PageSizeScript> = {}) {
  root?.unmount();
  document.getElementById('root')!.style.display = 'none';
  let host = document.getElementById('page-size-fixture');
  if (!host) {
    host = document.createElement('div');
    host.id = 'page-size-fixture';
    document.body.append(host);
  }
  root = createRoot(host);
  const script: PageSizeScript = {
    usage: usageOf(),
    report: reportOf(),
    refuseSave: false,
    reader: false,
    calls: { usage: 0, compact: 0, edit: 0, exported: 0 },
    ...first,
  };
  window.__pageSize = script;
  const binding: PageBinding = {
    subscribe(publish) {
      publish({ title: 'Release notes', source: '<h1>Full page</h1>' });
      return () => {};
    },
    async edit() {
      script.calls.edit++;
      if (script.refuseSave)
        throw new SaveRefused('COLAB_CAPACITY', 'Page is full: nothing was deleted.');
    },
    async export() {
      script.calls.exported++;
      throw new Error('No export in this fixture');
    },
    close() {},
    ...(script.reader
      ? {}
      : {
          async usage() {
            script.calls.usage++;
            return script.usage;
          },
          async compact() {
            script.calls.compact++;
            if (!script.report) throw new Error('Page reply timed out');
            script.usage = script.report.after;
            return script.report;
          },
        }),
  };
  const transport = localTransport('Review space', [
    {
      id: 'notes',
      title: 'Release notes',
      source: '<h1>Full page</h1>',
      sharing: 'private',
      binding,
    },
  ]);
  root.render(
    <RouterProvider router={createAppRouter({ ...transport, backendName: 'Studio Mac' })} />,
  );
  location.hash = '/pages/notes';
}
