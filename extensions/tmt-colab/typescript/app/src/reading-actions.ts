import type { Reading } from './questions.js';
import { currentReadingComment, readingComment, readingState } from './reading.js';
import { AdmittedView } from './review-actions.js';
import { resolvedByPerson } from './reviews.js';
import type { DiscussionRef, QuoteSelector, ThreadView } from './thread-records.js';
import type { ThreadStatusCoordinator } from './thread-status-coordinator.js';
import type { CommentContext, ThreadBinding } from './thread-store.js';

export interface ReadingOutcome {
  state: 'working' | 'failed' | 'uncertain' | 'settled';
  step: 'recording' | 'closing';
  warnings?: string[];
}
export interface ReadingActionsOptions {
  thread(id: string): ThreadView | undefined;
  selector(id: string): QuoteSelector | undefined;
  threadHolding(thread: DiscussionRef, message: DiscussionRef): ThreadView | undefined;
  discussion(): ThreadBinding | undefined;
  status(): ThreadStatusCoordinator | undefined;
  current(binding: ThreadBinding): boolean;
  title(): string;
  admitted: AdmittedView;
  changed(id: string, outcome: ReadingOutcome): void;
}

/** Only a trusted parent action calls this owner. Records and status use existing publication;
 * no render, reload or notification-retry path can call Mark read implicitly. */
export class ReadingActions {
  #inFlight = new Set<string>();
  #recorded = new Map<string, { binding: ThreadBinding; context: CommentContext }>();
  constructor(private options: ReadingActionsOptions) {}

  async mark(reading: Reading) {
    const binding = this.options.discussion();
    const status = this.options.status();
    const selector = this.options.selector(reading.id);
    if (
      !binding ||
      !status ||
      !selector ||
      !this.options.current(binding) ||
      this.#inFlight.has(reading.id)
    )
      return;
    const currentThread = this.options.thread(reading.id);
    if (readingState(reading, currentThread).kind === 'read') return;
    this.#inFlight.add(reading.id);
    const anchor = structuredClone(selector);
    const changed = (outcome: ReadingOutcome) => {
      if (this.options.current(binding)) this.options.changed(reading.id, outcome);
    };
    let step: ReadingOutcome['step'] = 'recording';
    try {
      changed({ state: 'working', step });
      const recorded = this.#recorded.get(reading.id);
      let context = recorded?.binding === binding ? recorded.context : undefined;
      const previous = currentThread && currentReadingComment(reading, currentThread.comments);
      if (!context && currentThread && previous)
        context = {
          thread: currentThread.ref,
          message: previous.ref,
          threadRevision: currentThread.revision,
          messageRevision: previous.revision,
        };
      if (!context) {
        const body = readingComment(reading, this.options.title(), `@${reading.agent}`);
        if (!body) throw new Error('Reading comment exceeds its bound');
        context = currentThread
          ? await binding.reply(currentThread.ref, body, currentThread.revision)
          : await binding.create(body, anchor);
        this.#recorded.set(reading.id, { binding, context });
      }
      step = 'closing';
      changed({ state: 'working', step });
      const read = () => this.options.threadHolding(context.thread, context.message);
      let thread = await this.options.admitted.until(read);
      if (!this.options.current(binding)) throw new Error('Page unavailable');
      if (thread.resolved && !resolvedByPerson(thread)) {
        await status.change(thread, false);
        thread = await this.options.admitted.until(() => {
          const next = read();
          return next && !next.resolved ? next : undefined;
        });
      }
      if (!this.options.current(binding)) throw new Error('Page unavailable');
      const result = resolvedByPerson(thread)
        ? undefined
        : await status.change(thread, true, { onlyAgentName: reading.agent });
      const warnings =
        result && 'warnings' in result
          ? result.warnings?.map((warning) => warning.name)
          : undefined;
      this.#recorded.delete(reading.id);
      changed({ state: 'settled', step, ...(warnings?.length ? { warnings } : {}) });
    } catch {
      changed({ state: step === 'recording' ? 'uncertain' : 'failed', step });
    } finally {
      this.#inFlight.delete(reading.id);
    }
  }
}
