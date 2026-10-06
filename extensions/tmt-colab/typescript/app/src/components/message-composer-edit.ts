import type { AgentDestination } from '../live-ask.js';
import type { RecipientKey } from '../message-recipient.js';

export type { RecipientKey } from '../message-recipient.js';
export type ComposerEdit = {
  value: string;
  /** Parent-selected identity is independent of message bytes or a mention token. */
  recipient?: RecipientKey;
  /** Optional editing presentation; invalidating a token does not clear the selected identity. */
  mention?: { key: RecipientKey; range: { start: number; end: number } };
};

export type MessageComposerProps = {
  edit: ComposerEdit;
  onChange(edit: ComposerEdit): void;
  label: string;
  placeholder?: string;
  disabled: boolean;
  autoFocus?: boolean;
  candidates?: readonly AgentDestination[];
  /** Optional explicit picker: selecting a stable key leaves plaintext unchanged. */
  recipientPickerLabel?: string;
  /** An intentional parent replacement starts a fresh editing/history lifetime. */
  resetKey?: string | number;
  onSubmit?(event: KeyboardEvent): void;
  onCancel?(event: KeyboardEvent): void;
};
