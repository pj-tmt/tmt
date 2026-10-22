import type { ActivityObservation } from './agent-activity.js';
import type { MessageStates } from './message-state.js';
import type { RemotePresence } from './remote-presence.js';
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
import type { ReadingDetails } from './reading-details.js';
import type { OwnState, Projection } from './fold-protocol.js';
import type { WorkAction } from './save.js';
export interface PageView extends Projection {
  /** The document is shown, but its threads and asks are still being verified: `threads`, `asks`
   * and `threadPresentations` are absent, never empty, so nothing unverified reads as verified. */
  readonly discussionPending?: boolean;
  readonly archived?: boolean;
  readonly ownData?: boolean;
  readonly own?: OwnState;
  readonly asks?: readonly PageAsk[];
  readonly threads?: readonly ThreadView[];
  readonly threadPresentations?: readonly ThreadPresentation[];
  readonly askUnavailable?: boolean;
  readonly agentActivity?: ActivityObservation;
  readonly messageStates?: MessageStates;
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
  /** Display-only projects from the owner store; absent where a source has none (fixtures, a reader). */
  readonly projects?: readonly string[];
  /** Pinned by the owner (display-only): the list leads with these, in the order the owner gives. */
  readonly pinned?: boolean;
  /** The owner's display-only summary (comment count, pending-item count, the creator); absent until derived. */
  readonly summary?: PageActivity;
  /** `shell` for a shell page; absent on an ordinary page and where the list does not say (the
   * owner list does not carry it yet, so only fixtures fill it). */
  readonly pageType?: Projection['pageType'];
}
export interface PageSnapshot extends PageSummary {
  /** The page's chosen layout (`meta.layout`); absent is `freeform`. */
  readonly layout?: Projection['layout'];
  /** Why a canvas page draws its own frame (`meta.layoutReason`). */
  readonly layoutReason?: Projection['layoutReason'];
  /** As on the view: the threads and asks follow through the binding once verified. */
  readonly discussionPending?: boolean;
  readonly shell?: Projection['shell'];
  /** The page's summary and task list (`meta.work`); never on a dashboard. */
  readonly work?: Projection['work'];
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
  readonly agentActivity?: ActivityObservation;
  readonly messageStates?: MessageStates;
}
/** What a Save did: `merged` when the page had changed meanwhile and the edit was merged into it. */
export interface SaveOutcome {
  merged: boolean;
}
/** What a work action made: the ID of a committed `task.add`, the page of a `task.convert`. */
export interface WorkOutcome {
  taskId?: string;
  /** The page a committed `task.convert` made and linked. */
  childPage?: string;
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
  /** Whether this device may change the page's work (the owner device's alone). */
  canWork?(): boolean;
  /** One closed work action, committed by the serve; the new value follows through `subscribe`.
   * Throws `SaveRefused` with the serve's code (`COLAB_WIP_LIMIT`, `COLAB_STALE_BASE`, …). */
  work?(action: WorkAction, body: Record<string, unknown>): Promise<WorkOutcome>;
  /** How full the page is, from the serve. Present only where this device may save. */
  usage?(): Promise<PageUsage>;
  /** Combine this serve's own stream and report what changed. Present only where `usage` is. */
  compact?(): Promise<CompactReport>;
  export(): Promise<ExportBundle>;
  close(): void;
}
export interface SpaceHome {
  readonly title: string;
  /** Every project of the space, those with no page too; absent where a source has none. */
  readonly projects?: readonly string[];
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
  /** Mounted-door snapshot only; absent for hosted and detached adapters. */
  readonly remotePresence?: RemotePresence;
  /** Display-only name from the mounted session device context. */
  readonly backendName?: string;
  /** Device-local unsent-draft persistence; absent means drafts stay in memory. */
  readonly drafts?: DraftStore;
  readonly management?: ManagementPort;
  /** The space's page list. A `signal` that aborts ends the read: a reader that leaves cancels it. */
  spaceHome(signal?: AbortSignal): Promise<SpaceHome>;
  /** Optional, owner-only cached reading metadata. No page subscription, fold or publication. */
  readingDetails?(id: string, signal: AbortSignal): Promise<ReadingDetails>;
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
        pages: [...pages.values()].map(({ id, title, sharing, pageType }) => ({
          id,
          title,
          sharing,
          ...(pageType ? { pageType } : {}),
        })),
      };
    },
    async page(id) {
      const page = pages.get(id);
      if (!page) throw new Error('Unknown page');
      return { ...page };
    },
  };
}
