import type { AskBinding, PageAsk } from './ask-panel.js';
import type { ExportBundle } from './export.js';
import type { ManagementPort } from './management.js';
import type { OwnState, Projection } from './fold-protocol.js';
export interface PageView extends Projection {
  readonly ownData?: boolean;
  readonly own?: OwnState;
  readonly asks?: readonly PageAsk[];
  readonly askUnavailable?: boolean;
}
export interface PageSummary {
  readonly id: string;
  readonly title: string;
  readonly sharing: 'private' | 'link' | 'public';
  readonly archived?: boolean;
  readonly retentionDays?: number | null;
}
export interface PageSnapshot extends PageSummary {
  readonly source: string;
  readonly binding?: PageBinding;
  readonly ownData?: boolean;
  readonly own?: OwnState;
  readonly asks?: readonly PageAsk[];
  readonly askUnavailable?: boolean;
}
export interface PageBinding {
  readonly ask?: AskBinding;
  reconnect?(): Promise<boolean>;
  subscribe(publish: (value: PageView) => void, failed: (error: Error) => void): () => void;
  edit(source: string, base: string): Promise<void>;
  export(): Promise<ExportBundle>;
  close(): void;
}
export interface SpaceHome {
  readonly title: string;
  readonly pages: readonly PageSummary[];
}

/** App data port. Mounted adapters own authentication and admission;
 * neither HTML nor the renderer receives that adapter or its capabilities. */
export interface PageTransport {
  readonly management?: ManagementPort;
  spaceHome(): Promise<SpaceHome>;
  page(id: string, signal?: AbortSignal): Promise<PageSnapshot>;
}

/** Detached, read-only local fixture adapter; not a persisted or authenticated backend. */
export function localTransport(title: string, input: readonly PageSnapshot[]): PageTransport {
  const pages = new Map(input.map((page) => [page.id, Object.freeze({ ...page })]));
  if (pages.size !== input.length) throw new Error('Duplicate local page ID');
  return {
    async spaceHome() {
      return {
        title,
        pages: [...pages.values()].map(({ id, title, sharing }) => ({ id, title, sharing })),
      };
    },
    async page(id) {
      const page = pages.get(id);
      if (!page) throw new Error('Unknown page');
      return { ...page };
    },
  };
}
