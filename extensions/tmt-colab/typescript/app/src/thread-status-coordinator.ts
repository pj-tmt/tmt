import { requireValue } from '@tmt/colab-client';
import type { PageAsk } from './ask-panel.js';
import type { AgentDestination } from './live-ask.js';
import type { ThreadView } from './thread-records.js';
import type { ThreadBinding, StatusChange } from './thread-store.js';
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
  const recipients = new Map<string, ThreadRecipient>();
  const warnings = new Map<string, MentionWarning>();
  for (const comment of thread.comments.filter((value) => !value.deleted)) {
    const pinned: typeof current = new Map();
    for (const ask of asks) {
      if (ask.thread === thread.threadId && ask.messageIds?.includes(comment.messageId))
        put(pinned, ask.agentName, ask);
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
  ): Promise<NotificationAdoption>;
}
/** Explicit parent action only. The status and recipient operation IDs are
 * durable before sequential notification attempts. No open/reload/retry path. */
export class ThreadStatusCoordinator {
  #changing = new Map<string, Promise<StatusChange & { warnings?: MentionWarning[] }>>();
  constructor(private options: ThreadStatusCoordinatorOptions) {}
  change(thread: ThreadView, resolved: boolean) {
    const key = discussionRefKey(thread.ref);
    const pending = this.#changing.get(key);
    if (pending) return pending;
    const captured = structuredClone(thread);
    const asks = structuredClone([...this.options.asks()]);
    const task = this.#change(captured, asks, resolved).finally(() => this.#changing.delete(key));
    this.#changing.set(key, task);
    return task;
  }
  async #change(thread: ThreadView, asks: readonly PageAsk[], resolved: boolean) {
    requireValue(isStatusThread(thread));
    if (thread.resolved === resolved) return { changed: false } as const;
    let destinations: readonly AgentDestination[] = [];
    if (resolved) {
      try {
        destinations = await this.options.destinations();
      } catch {
        // Signed recipients stay identifiable when the directory is unavailable.
        // Names without a binding remain unknown, never guessed.
      }
    }
    const frozen = resolved
      ? freezeThreadRecipients(thread, asks, destinations)
      : { recipients: [], warnings: [] };
    const result = await this.options.binding.setStatus(
      thread.ref,
      thread.status?.ref ?? null,
      resolved,
      frozen.recipients,
    );
    if (!result.changed) return result;
    for (const recipient of result.status.recipients) {
      try {
        const outcome = await this.options.notify(result.status, recipient, thread);
        if (!outcome.adopted)
          await this.options.binding.notificationFailed(
            result.status,
            recipient.operationId,
            outcome.reason,
          );
      } catch {
        // A published operation without an admitted ledger/failure outcome is
        // uncertain. Never treat an exception as permission to resend; other
        // independently frozen recipients still get their own single attempt.
      }
    }
    return { ...result, warnings: frozen.warnings };
  }
}
