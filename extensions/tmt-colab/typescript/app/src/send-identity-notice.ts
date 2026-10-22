import type { SendIdentity } from './ask-remote.js';

/** The one line a page session tells the person about how their send was attributed (#4255).
 * In memory only: it is shown once per page session and never stored, and it never changes,
 * blocks or retries a send. */
export class SendIdentityNotice {
  #current: SendIdentity | null = null;
  #told = false;
  #listeners = new Set<() => void>();
  /** The first attribution worth saying wins the session; later ones are ignored. */
  show = (identity: SendIdentity): void => {
    if (this.#told) return;
    this.#told = true;
    this.#current = identity;
    this.#listeners.forEach((listener) => listener());
  };
  dismiss = (): void => {
    if (!this.#current) return;
    this.#current = null;
    this.#listeners.forEach((listener) => listener());
  };
  current = (): SendIdentity | null => this.#current;
  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };
}

/** Core's message without its own `CODE: ` prefix (only when the code is this notice's reason, which
 * stays in the line's `data-reason`), first letter capitalised, so the person reads a sentence. */
export function anonymousText(identity: Extract<SendIdentity, { state: 'anonymous' }>): string {
  const prefix = `${identity.reason}: `;
  const rest = identity.message.startsWith(prefix)
    ? identity.message.slice(prefix.length)
    : identity.message;
  return rest.charAt(0).toUpperCase() + rest.slice(1);
}

/** The command Core's message names, rendered as selectable code. */
export const USER_NAME_COMMAND = 'tmt config set user.name <name>';
/** Core's message split around the command, so the command can be shown as code. */
export function messageParts(message: string): { before: string; command: boolean; after: string } {
  const at = message.indexOf(USER_NAME_COMMAND);
  if (at < 0) return { before: message, command: false, after: '' };
  return {
    before: message.slice(0, at),
    command: true,
    after: message.slice(at + USER_NAME_COMMAND.length),
  };
}
