import type { AskBinding, PageAsk } from './ask-panel.js';
import { sendCommentAsks, type MessageSendFailure } from './comment-send.js';
import type { AgentDestination } from './live-ask.js';
import { recipientEdit } from './message-recipient.js';
import type { Review } from './questions.js';
import {
  answeredComment,
  confirmComment,
  needsChangeComment,
  resolvedByPerson,
} from './reviews.js';
import type { QuoteSelector, ThreadView, DiscussionRef } from './thread-records.js';
import type { ThreadStatusCoordinator } from './thread-status-coordinator.js';
import { captureConversation, type CommentContext, type ThreadBinding } from './thread-store.js';

/** How long an action waits for the page's admitted view to show a record it just published. */
export const ADMITTED_WAIT_MS = 10_000;

/** Resolves when the admitted discussion view shows what an action just published. Publication
 * resolves before the fold does, and the next step needs the thread as the view holds it. */
export class AdmittedView {
  #waiters = new Set<() => void>();
  constructor(private timeoutMs = ADMITTED_WAIT_MS) {}
  /** The view changed: let every waiter look again. */
  update() {
    for (const check of this.#waiters) check();
  }
  until<T>(read: () => T | undefined): Promise<T> {
    return new Promise((resolve, reject) => {
      const done = () => {
        clearTimeout(timer);
        this.#waiters.delete(check);
      };
      const check = () => {
        const value = read();
        if (value === undefined) return;
        done();
        resolve(value);
      };
      const timer = setTimeout(() => {
        done();
        reject(new Error('The page did not show the record in time'));
      }, this.timeoutMs);
      this.#waiters.add(check);
      check();
    });
  }
}

export interface ReviewOutcome {
  state: 'working' | 'failed' | 'uncertain' | 'settled';
  /** Where it stopped. `closing` means the comment is recorded and only the status is missing. */
  step?: 'recording' | 'closing';
  /** Asks the Needs-change note could not deliver, for the existing retry action. */
  failures?: MessageSendFailure[];
  /** Mentions that could not become recipients of the status notification. */
  warnings?: string[];
}

export interface ReviewActionsOptions {
  /** The section's canonical thread in the admitted view, if it has one. */
  thread(review: string): ThreadView | undefined;
  /** The section heading's quote, from the renderer's normal resolver. */
  selector(review: string): QuoteSelector | undefined;
  /** The asking agent, resolved through the directory; undefined while it is still loading. */
  recipients(review: Review): readonly AgentDestination[] | undefined;
  /** Whether the reader's answers now answer every question inside the section. */
  answered(review: Review): boolean;
  /** The thread `thread` as the admitted view holds it, only once it shows `message`. */
  threadHolding(thread: DiscussionRef, message: DiscussionRef): ThreadView | undefined;
  discussion(): ThreadBinding | undefined;
  ask(): AskBinding | undefined;
  asks(): readonly PageAsk[];
  status(): ThreadStatusCoordinator | undefined;
  current(discussion: ThreadBinding, ask: AskBinding | undefined): boolean;
  admitted: AdmittedView;
  title(): string;
  url(): string;
  changed(review: string, outcome: ReviewOutcome): void;
}

/**
 * The explicit, trusted section actions. Each one is a normal comment (and, for a change request,
 * the existing Ask) plus the existing person Resolve or Reopen. Nothing runs on render, sync or
 * reload, and page HTML cannot reach this class: it only ever receives IDs the parent admitted.
 */
export class ReviewActions {
  #inFlight = new Set<string>();
  /** A comment this tab recorded whose Resolve did not land: a retry publishes only the Resolve,
   * instead of recording the confirmation twice. */
  #recorded = new Map<string, CommentContext>();
  constructor(private options: ReviewActionsOptions) {}

  /** The thread as the admitted view holds it once it shows the recorded message. */
  #admittedThread(context: CommentContext) {
    return this.options.admitted.until(() =>
      this.options.threadHolding(context.thread, context.message),
    );
  }

  /** Capture what an action needs, or nothing when it may not run now. A comment that names the
   * asking agent needs the directory read; closing on answers does not. */
  #begin(review: Review, mentions: boolean) {
    const discussion = this.options.discussion();
    const ask = this.options.ask();
    const status = this.options.status();
    const selector = this.options.selector(review.id);
    const recipients = mentions ? this.options.recipients(review) : [];
    if (
      this.#inFlight.has(review.id) ||
      !discussion ||
      !status ||
      !selector ||
      !recipients ||
      !this.options.current(discussion, ask)
    )
      return;
    this.#inFlight.add(review.id);
    return {
      discussion,
      ask,
      status,
      selector: structuredClone(selector),
      recipients: structuredClone([...recipients]),
      thread: this.options.thread(review.id),
      title: this.options.title(),
      url: this.options.url(),
    };
  }

  #record(
    c: { discussion: ThreadBinding; thread?: ThreadView; selector: QuoteSelector },
    body: string,
  ): Promise<CommentContext> {
    return c.thread
      ? c.discussion.reply(c.thread.ref, body, c.thread.revision)
      : c.discussion.create(body, c.selector);
  }

  /** Publish the person Resolve on the thread that holds `context`. An agent's resolution leaves
   * the status as the agent labelled it, so the thread is reopened first and then resolved. */
  async #resolve(status: ThreadStatusCoordinator, context: CommentContext) {
    let thread = await this.#admittedThread(context);
    const warnings: string[] = [];
    if (thread.resolved && !resolvedByPerson(thread)) {
      await status.change(thread, false);
      thread = await this.options.admitted.until(() => {
        const next = this.options.threadHolding(context.thread, context.message);
        return next && !next.resolved ? next : undefined;
      });
    }
    if (!resolvedByPerson(thread)) {
      const result = await status.change(thread, true);
      for (const warning of ('warnings' in result && result.warnings) || [])
        warnings.push(warning.name);
    }
    return warnings;
  }

  /** `✓ Confirm and close`: record the confirmation (with the asking agent named, so the person
   * Resolve tells it once), then resolve the thread. */
  async confirm(review: Review, note: string) {
    const c = this.#begin(review, true);
    if (!c) return;
    const changed = (outcome: ReviewOutcome) => {
      if (this.options.discussion() === c.discussion) this.options.changed(review.id, outcome);
    };
    let step: ReviewOutcome['step'] = 'recording';
    try {
      changed({ state: 'working', step });
      let context = this.#recorded.get(review.id);
      if (!context) {
        const body = confirmComment(review, note, c.title, recipientEdit(c.recipients).value);
        if (!body) throw new Error('The note is not valid');
        context = await this.#record(c, body);
        this.#recorded.set(review.id, context);
      }
      step = 'closing';
      changed({ state: 'working', step });
      if (!this.options.current(c.discussion, c.ask)) throw new Error('Page unavailable');
      const warnings = await this.#resolve(c.status, context);
      this.#recorded.delete(review.id);
      changed({ state: 'settled', ...(warnings.length ? { warnings } : {}) });
    } catch {
      changed({ state: step === 'closing' ? 'failed' : 'uncertain', step });
    } finally {
      this.#inFlight.delete(review.id);
    }
  }

  /** `✎ Needs change…`: record the note and ask the agent through the existing Ask. The thread
   * stays open. */
  async requestChange(review: Review, note: string) {
    const c = this.#begin(review, true);
    if (!c) return;
    const changed = (outcome: ReviewOutcome) => {
      if (this.options.discussion() === c.discussion) this.options.changed(review.id, outcome);
    };
    let recorded = false;
    try {
      changed({ state: 'working', step: 'recording' });
      const body = needsChangeComment(review, note, c.title, recipientEdit(c.recipients).value);
      if (!body) throw new Error('The note is not valid');
      // The earlier conversation is captured before this note joins it.
      const conversation = captureConversation(c.thread, this.options.asks());
      const context = await this.#record(c, body);
      recorded = true;
      const failures = await sendCommentAsks(c.ask, context, c.recipients, {
        title: c.title,
        url: c.url,
        quote: c.selector.exact,
        comment: body,
        conversation,
      });
      changed({
        state: failures.some((failure) => failure.uncertain)
          ? 'uncertain'
          : failures.length
            ? 'failed'
            : 'settled',
        ...(failures.length ? { failures } : {}),
      });
    } catch {
      // Before the record landed nothing was sent; after it, a failed Ask is reported per
      // recipient above, so only an unexpected error reaches this point.
      changed({ state: recorded ? 'uncertain' : 'failed', step: 'recording' });
    } finally {
      this.#inFlight.delete(review.id);
    }
  }

  /** `Reopen` on a closed section: the ordinary person Reopen, which notifies no one. */
  async reopen(review: Review) {
    const status = this.options.status();
    const thread = this.options.thread(review.id);
    if (this.#inFlight.has(review.id) || !status || !thread || !resolvedByPerson(thread)) return;
    this.#inFlight.add(review.id);
    this.options.changed(review.id, { state: 'working', step: 'closing' });
    try {
      await status.change(thread, false);
      this.options.changed(review.id, { state: 'settled' });
    } catch {
      this.options.changed(review.id, { state: 'failed', step: 'closing' });
    } finally {
      this.#inFlight.delete(review.id);
    }
  }

  /**
   * The reader's trusted Send has just recorded answers to questions inside `review`. When they
   * now answer every question there, close the section in the same activation: an opening comment
   * if the thread is new, then the person Resolve. A failure leaves the section waiting with
   * Confirm still available; nothing is closed by deriving it.
   */
  async closeAnswered(review: Review, answers: readonly CommentContext[]) {
    const c = this.#begin(review, false);
    if (!c) return;
    const changed = (outcome: ReviewOutcome) => {
      if (this.options.discussion() === c.discussion) this.options.changed(review.id, outcome);
    };
    let step: ReviewOutcome['step'] = 'recording';
    try {
      // The answers are in other threads: wait until the view shows them, then decide.
      for (const answer of answers) await this.#admittedThread(answer);
      if (!this.options.answered(review)) return;
      const current = this.options.thread(review.id);
      if (resolvedByPerson(current)) return;
      changed({ state: 'working', step });
      const body = answeredComment(review, c.title);
      if (!body) throw new Error('The comment is not valid');
      const context = await this.#record({ ...c, thread: current }, body);
      step = 'closing';
      changed({ state: 'working', step });
      await this.#resolve(c.status, context);
      changed({ state: 'settled' });
    } catch {
      changed({ state: 'failed', step });
    } finally {
      this.#inFlight.delete(review.id);
    }
  }
}
