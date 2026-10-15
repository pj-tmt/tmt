import type { CompactReport, PageUsage } from './page-usage.js';
import type { attachment } from '@tmt/colab-client';
import type { ThreadPresentation } from './thread-status-presentation.js';
import type { ThreadStatusCoordinator } from './thread-status-coordinator.js';
import type { ThreadBinding } from './thread-store.js';
import type { DocumentFiles } from './document-files.js';
import type { DiscussionRef, ThreadView } from './thread-records.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import type { ExportBundle } from './export.js';
import type { ManagementPort } from './management.js';
import type { OwnState, Projection } from './fold-protocol.js';
export interface PageView extends Projection {
  readonly archived?: boolean;
  readonly ownData?: boolean;
  readonly own?: OwnState;
  readonly asks?: readonly PageAsk[];
  readonly threads?: readonly ThreadView[];
  readonly threadPresentations?: readonly ThreadPresentation[];
  readonly askUnavailable?: boolean;
}
import type { PageActivity, PageId } from './bootstrap.js';
import type { DraftStore } from './draft-store.js';
import type { ExpiryInfo } from './expiry.js';
export interface PageSummary extends Partial<ExpiryInfo> {
  readonly id: string;
  readonly title: string;
  /** No usable browser-local title has been cached yet. */
  readonly titleMissing?: boolean;
  readonly sharing: 'private' | 'link' | 'public';
  readonly archived?: boolean;
  /** Display-only labels from the owner store; absent where a source has none (fixtures, a reader). */
  readonly labels?: readonly string[];
  /** The owner's display-only summary (comment count, a proposal awaiting you, the creator); absent until derived. */
  readonly summary?: PageActivity;
}
export interface PageSnapshot extends PageSummary {
  readonly originalAuthor?: Projection['originalAuthor'];
  readonly publisherAgent?: Projection['publisherAgent'];
  readonly source: string;
  readonly binding?: PageBinding;
  readonly ownData?: boolean;
  readonly own?: OwnState;
  readonly asks?: readonly PageAsk[];
  readonly threads?: readonly ThreadView[];
  readonly threadPresentations?: readonly ThreadPresentation[];
  readonly askUnavailable?: boolean;
}
/** What a Save did: `merged` when the page had changed meanwhile and the edit was merged into it. */
export interface SaveOutcome {
  merged: boolean;
}
export interface PageBinding {
  readonly ask?: AskBinding;
  readonly discussion?: ThreadBinding;
  readonly status?: ThreadStatusCoordinator;
  /** Files attached to the page document. */
  readonly files?: DocumentFiles;
  markThreadStatusSeen?(thread: DiscussionRef): void;
  reconnect?(): Promise<boolean>;
  subscribe(publish: (value: PageView) => void, failed: (error: Error) => void): () => void;
  /** Save the source the editor opened as `base` changed into `source`, merging it into a page
   * that moved meanwhile; a typed change also sets or removes document attachment references. */
  edit(source: string, base: string, attachments?: attachment.DocumentChange): Promise<SaveOutcome>;
  /** How full the page is, from the serve. Present only where this device may save. */
  usage?(): Promise<PageUsage>;
  /** Combine this serve's own stream and report what changed. Present only where `usage` is. */
  compact?(): Promise<CompactReport>;
  export(): Promise<ExportBundle>;
  close(): void;
}
export interface SpaceHome {
  readonly title: string;
  readonly pages: readonly PageSummary[];
  readonly pageIds?: readonly PageId[];
}

/** Shared across a bounded group of read-only page folds, including failed reads. */
export interface TitleReadBudget {
  remainingBytes: number;
}

/** App data port. Mounted adapters own authentication and admission;
 * neither HTML nor the renderer receives that adapter or its capabilities. */
export interface PageTransport {
  /** Display-only name from the mounted session device context. */
  readonly backendName?: string;
  /** Device-local unsent-draft persistence; absent means drafts stay in memory. */
  readonly drafts?: DraftStore;
  readonly management?: ManagementPort;
  spaceHome(): Promise<SpaceHome>;
  /** A verified title only: no editor, discussion or live subscription. */
  readTitle?(id: string, budget: TitleReadBudget, signal: AbortSignal): Promise<string | undefined>;
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
