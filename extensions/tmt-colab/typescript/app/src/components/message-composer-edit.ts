import type { AgentDestination } from '../live-ask.js';
import type { RecipientKey } from '../message-recipient.js';

export type { RecipientKey } from '../message-recipient.js';
export type ComposerEdit = {
  value: string;
  /** Intact editing tokens; the parent revalidates every key against the current directory. */
  mentions?: { key: RecipientKey; range: { start: number; end: number } }[];
  /** False for an untouched creator seed; edited/restored blanks retain a removed default. */
  edited?: boolean;
};

export type MessageComposerProps = {
  edit: ComposerEdit;
  onChange(edit: ComposerEdit): void;
  label: string;
  placeholder?: string;
  disabled: boolean;
  autoFocus?: boolean;
  candidates?: readonly AgentDestination[];
  describedBy?: string;
  /** An intentional parent replacement starts a fresh editing/history lifetime. */
  resetKey?: string | number;
  onSubmit?(event: KeyboardEvent, edit: ComposerEdit): void;
  onCancel?(event: KeyboardEvent): void;
};
