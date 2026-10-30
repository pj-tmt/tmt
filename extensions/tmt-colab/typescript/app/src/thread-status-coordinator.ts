import { requireValue } from '@tmt/colab-client';
import type { PageAsk } from './ask-panel.js';
import type { AgentDestination } from './live-ask.js';
import type { ThreadView } from './thread-records.js';
import { asksAbout, type ThreadBinding, type StatusChange } from './thread-store.js';
import {
  discussionRefKey,
  type ThreadRecipient,
  type ThreadStatusView,
  type ThreadNotificationRecord,
} from './thread-status.js';
import { isStatusThread } from './thread-status-view.js';

const recipientKey = (value: { machine: string; agent: string }) =>
  `${value.machine}:${value.agent}`;
export interface MentionWarning {
  name: string;
  reason: 'unknown' | 'ambiguous';
}

/** Authenticated discussion/Ask projections and the current agents list. A signed
 * prior Ask pins a mentioned name to its recipient UUID after a rename. A name
 * alone must uniquely match; replies are not required. */
export function freezeThreadRecipients(
  thread: ThreadView,
  asks: readonly PageAsk[],
  destinations: readonly AgentDestination[],
  onlyAgentName?: string,
) {
  const current = new Map<
    string,
    Map<string, { machine: string; agent: string; agentName: string }>
  >();
  const put = (
    map: typeof current,
    name: string,
    recipient: { machine: string; agent: string; agentName: string },
  ) => {
    const matches = map.get(name) ?? new Map();
    matches.set(recipientKey(recipient), recipient);
    map.set(name, matches);
  };
  for (const destination of destinations) put(current, destination.agentName, destination);
  // Reading actions resolve their authored name from the current directory only. The
  // passage's other mentions and prior Asks must not expand this explicit recipient.
  if (onlyAgentName !== undefined) {
    const matches = current.get(onlyAgentName);
    if (matches?.size !== 1)
      return {
        recipients: [] as ThreadRecipient[],
        warnings: [
          {
            name: onlyAgentName,
            reason: matches?.size ? ('ambiguous' as const) : ('unknown' as const),
          },
        ],
      };
    const value = [...matches.values()][0];
    return {
      recipients: [
        {
          machine: value.machine,
          agent: value.agent,
          agentName: onlyAgentName,
          operationId: crypto.randomUUID(),
        },
      ],
      warnings: [] as MentionWarning[],
    };
  }
  const recipients = new Map<string, ThreadRecipient>();
  const warnings = new Map<string, MentionWarning>();
  for (const comment of thread.comments.filter((value) => !value.deleted)) {
    const pinned: typeof current = new Map();
    for (const ask of asks) {
      if (asksAbout(ask, thread, comment)) put(pinned, ask.agentName, ask);
    }
    const names = [...new Set([...current.keys(), ...pinned.keys()])]
      .filter(Boolean)
      .sort((a, b) => b.length - a.length || (a < b ? -1 : 1));
    const pattern = /@/g;
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(comment.body))) {
      const index = match.index;
      if (index > 0 && /[\p{L}\p{N}_]/u.test(comment.body[index - 1])) continue;
      const remainder = comment.body.slice(index + 1);
      const name = names.find(
        (value) =>
          remainder.startsWith(value) &&
          /^(?:$|[\s,.!?;:()[\]{}])/u.test(remainder.slice(value.length)),
      );
      if (!name) {
        const unknown = /^([^\s,.!?;:()[\]{}]+)/u.exec(remainder)?.[1];
        if (unknown) warnings.set(unknown, { name: unknown, reason: 'unknown' });
        continue;
      }
      const candidates = pinned.get(name) ?? current.get(name)!;
      if (candidates.size !== 1) {
        warnings.set(name, { name, reason: 'ambiguous' });
        continue;
      }
      const value = [...candidates.values()][0];
      const key = recipientKey(value);
      if (!recipients.has(key))
        recipients.set(key, {
          machine: value.machine,
          agent: value.agent,
          agentName: name,
          operationId: crypto.randomUUID(),
        });
      pattern.lastIndex = index + 1 + name.length;
    }
  }
  requireValue(recipients.size <= 1000);
  return {
    recipients: [...recipients.values()].sort((a, b) =>
      recipientKey(a) < recipientKey(b) ? -1 : 1,
    ),
    warnings: [...warnings.values()].sort((a, b) => (a.name < b.name ? -1 : 1)),
  };
}

export type NotificationAdoption =
  | { adopted: true }
  | { adopted: false; reason: ThreadNotificationRecord['reason'] };
export interface ThreadStatusCoordinatorOptions {
  binding: Pick<ThreadBinding, 'setStatus' | 'notificationFailed'>;
  asks(): readonly PageAsk[];
  destinations(): Promise<readonly AgentDestination[]>;
  notify(
    status: ThreadStatusView,
    recipient: ThreadRecipient,
    captured: ThreadView,
    destinations: readonly AgentDestination[],
  ): Promise<NotificationAdoption>;
}
/** Explicit parent action only. The status and recipient operation IDs are
 * durable before sequential notification attempts. No open/reload/retry path. */
export class ThreadStatusCoordinator {
  #changing = new Map<
    string,
    {
      resolved: boolean;
      generation: symbol;
      task: Promise<StatusChange & { warnings?: MentionWarning[] }>;
    }
  >();
  constructor(private options: ThreadStatusCoordinatorOptions) {}
  change(thread: ThreadView, resolved: boolean, options?: { onlyAgentName: string }) {
    const key = discussionRefKey(thread.ref);
    const pending = this.#changing.get(key);
    if (pending?.resolved === resolved) return pending.task;
    const generation = Symbol();
    const captured = structuredClone(thread);
    const asks = structuredClone([...this.options.asks()]);
    const task = Promise.resolve().then(() =>
      this.#change(key, generation, captured, asks, resolved, options?.onlyAgentName),
    );
    this.#changing.set(key, { resolved, generation, task });
    return task;
  }
  async #change(
    key: string,
    generation: symbol,
    thread: ThreadView,
    asks: readonly PageAsk[],
    resolved: boolean,
    onlyAgentName?: string,
  ) {
    let notifying = false;
    try {
      requireValue(isStatusThread(thread));
      if (thread.resolved === resolved) return { changed: false } as const;
      // Start directory I/O before the binding acquires its lock and admits the writer.
      // Prior signed IDs still identify recipients if the directory is unavailable.
      const destinations = resolved
        ? this.options.destinations().catch(() => [])
        : Promise.resolve([]);
      const frozen = destinations.then((values) =>
        resolved
          ? freezeThreadRecipients(thread, asks, values, onlyAgentName)
          : { recipients: [], warnings: [] },
      );
      const result = await this.options.binding.setStatus(
        thread.ref,
        thread.status?.ref ?? null,
        resolved,
        frozen.then((value) => value.recipients),
      );
      if (!result.changed) return result;
      notifying = true;
      void this.#notify(key, generation, result.status, thread, await destinations).finally(() => {
        if (this.#changing.get(key)?.generation === generation) this.#changing.delete(key);
      });
      return { ...result, warnings: (await frozen).warnings };
    } finally {
      if (!notifying && this.#changing.get(key)?.generation === generation)
        this.#changing.delete(key);
    }
  }
  async #notify(
    key: string,
    generation: symbol,
    status: ThreadStatusView,
    thread: ThreadView,
    destinations: readonly AgentDestination[],
  ) {
    for (const recipient of status.recipients) {
      // A newer explicit action supersedes this task, never the person's status write.
      if (this.#changing.get(key)?.generation !== generation) return;
      try {
        const outcome = await this.options.notify(status, recipient, thread, destinations);
        if (!outcome.adopted)
          await this.options.binding.notificationFailed(
            status,
            recipient.operationId,
            outcome.reason,
          );
      } catch {
        // Unknown adoption never permits a resend; independent recipients get one attempt each.
      }
    }
  }
}
