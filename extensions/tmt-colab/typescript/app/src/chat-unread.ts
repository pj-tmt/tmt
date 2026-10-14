import type { ReplyNotification } from './reply-notifications.js';

/** Device-local presentation position only; never a signed receipt or authority.
 * Identity membership, rather than a timestamp/index, admits late verified replies. */
export class ChatSeen {
  private seen = new Set<string>();
  private key: string;
  constructor(
    scope: string,
    private storage?: Pick<Storage, 'getItem' | 'setItem'>,
  ) {
    this.key = `tmt-colab:chat-seen:${scope}`;
    try {
      const value: unknown = JSON.parse(storage?.getItem(this.key) ?? '[]');
      if (Array.isArray(value) && value.every((id) => typeof id === 'string'))
        this.seen = new Set(value);
    } catch {
      // Missing/malformed storage is unread, not a false acknowledgement.
    }
  }
  unread(replies: Iterable<ReplyNotification>) {
    return [...replies].some((reply) => reply.destination === 'Chat' && !this.seen.has(reply.id));
  }
  read(replies: Iterable<ReplyNotification>) {
    let changed = false;
    for (const reply of replies)
      if (reply.destination === 'Chat' && !this.seen.has(reply.id)) {
        this.seen.add(reply.id);
        changed = true;
      }
    if (changed)
      try {
        this.storage?.setItem(this.key, JSON.stringify([...this.seen]));
      } catch {
        // Keep this tab's position if storage is unavailable; no write retries.
      }
    return changed;
  }
}
