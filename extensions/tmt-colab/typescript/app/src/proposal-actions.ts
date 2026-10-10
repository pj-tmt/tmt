import type { AskBinding, PageAsk } from './ask-panel.js';
import type { AskAgainInput } from './ask-again.js';
import type { ThreadView } from './thread-records.js';
import { captureConversation, type ThreadBinding } from './thread-store.js';

export interface ProposalOutcome {
  state: 'deciding' | 'notifying' | 'failed' | 'uncertain' | 'settled';
  committed?: boolean;
  failure?: {
    input: AskAgainInput;
    recipient: { machine: string; agent: string };
    uncertain: boolean;
  };
}

/** One parent activation owns the final decision and its explicit Ask. A final
 * decision is never rolled back or retried, including a failed notification. */
export class ProposalActions {
  #inFlight = new Set<string>();
  constructor(
    private options: {
      thread(id: string): ThreadView | undefined;
      discussion(): ThreadBinding | undefined;
      ask(): AskBinding | undefined;
      asks(): readonly PageAsk[];
      current(discussion: ThreadBinding, ask: AskBinding | undefined): boolean;
      title(): string;
      url(): string;
      changed(id: string, outcome: ProposalOutcome): void;
    },
  ) {}
  async decide(id: string, decision: 'approved' | 'declined') {
    const thread = this.options.thread(id);
    const discussion = this.options.discussion();
    const ask = this.options.ask();
    if (
      this.#inFlight.has(id) ||
      !discussion?.decideProposal ||
      !thread?.proposal ||
      thread.deleted ||
      thread.decision ||
      !this.options.current(discussion, ask)
    )
      return;
    this.#inFlight.add(id);
    const captured = structuredClone({
      thread,
      title: this.options.title(),
      url: this.options.url(),
      conversation: captureConversation(thread, this.options.asks()),
    });
    let committed = false;
    let input: AskAgainInput | undefined;
    const recipient = {
      machine: captured.thread.proposal!.proposer.machineId,
      agent: captured.thread.proposal!.proposer.agentId,
    };
    const changed = (outcome: ProposalOutcome) => {
      if (this.options.discussion() === discussion) this.options.changed(id, outcome);
    };
    changed({ state: 'deciding' });
    try {
      await discussion.decideProposal(captured.thread.ref, decision);
      committed = true;
      changed({ state: 'notifying', committed });
      if (!this.options.current(discussion, ask)) throw new Error('Page unavailable');
      const comment = `${decision === 'approved' ? 'Approved' : 'Declined'}: ${captured.thread.proposal!.title}`;
      const origin = await discussion.reply(captured.thread.ref, comment, captured.thread.revision);
      input = {
        quote: '',
        comment,
        title: captured.title,
        url: captured.url,
        context: { ...origin, conversation: captured.conversation },
      };
      if (!ask || !this.options.current(discussion, ask)) throw new Error('Ask unavailable');
      const matches = (await ask.destinations()).filter(
        (d) => d.machine === recipient.machine && d.agent === recipient.agent,
      );
      if (matches.length !== 1 || !this.options.current(discussion, ask))
        throw new Error('Recipient unavailable');
      const attempt = await ask.prepare({ ...input, destination: matches[0] });
      if (!this.options.current(discussion, ask)) throw new Error('Page unavailable');
      try {
        const outcome = await attempt.send();
        const uncertain = outcome.adopted !== false && outcome.adopted !== true;
        changed({
          state: uncertain ? 'uncertain' : outcome.adopted === false ? 'failed' : 'settled',
          committed,
          ...(outcome.adopted !== true ? { failure: { input, recipient, uncertain } } : {}),
        });
      } catch {
        changed({ state: 'uncertain', committed, failure: { input, recipient, uncertain: true } });
      }
    } catch {
      changed({
        state: committed ? 'failed' : 'uncertain',
        committed,
        ...(input ? { failure: { input, recipient, uncertain: false } } : {}),
      });
    }
    // Keep the activation fence: publication errors can have an unknown outcome.
    // Refresh/reload may re-admit records, but never resumes this notification.
  }
}
