import type { ReactNode } from 'react';
import type { AgentDestination } from '../live-ask.js';
import type { RecipientKey } from '../message-recipient.js';

export type { RecipientKey } from '../message-recipient.js';
export type ComposerEdit = {
  value: string;
  /** Intact editing tokens; the parent revalidates every key against the current directory. */
  mentions?: { key: RecipientKey; range: { start: number; end: number }; defaultCreator?: true }[];
  /** False for an untouched creator seed; edited/restored blanks retain a removed default. */
  edited?: boolean;
};

/** One parent request consumed by the mounted editor without resetting its draft. */
export type ComposerActivation = { id: string; insertMention: boolean; consumed(): void };

export type MessageComposerProps = {
  edit: ComposerEdit;
  onChange(edit: ComposerEdit): void;
  label: string;
  /** Optional presentation below text inside the shared input. */
  accessory?: ReactNode;
  placeholder?: string;
  disabled: boolean;
  autoFocus?: boolean;
  activation?: ComposerActivation;
  candidates?: readonly AgentDestination[];
  /** Explicit page preference transitions only; fresh defaults remain parent-owned. */
  creatorDefault?: { key: RecipientKey; label: string; enabled: boolean; revision: number };
  describedBy?: string;
  /** An intentional parent replacement starts a fresh editing/history lifetime. */
  resetKey?: string | number;
  onSubmit?(event: KeyboardEvent, edit: ComposerEdit): void;
  onCancel?(event: KeyboardEvent): void;
};
