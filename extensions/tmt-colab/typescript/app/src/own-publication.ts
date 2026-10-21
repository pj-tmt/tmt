import type { OwnRecord, OwnRoot } from './fold-protocol.js';

const kinds: Record<OwnRoot, readonly string[]> = {
  threads: ['thread'],
  intents: ['ask', 'attachment-publication', 'edit-request'],
  messages: ['comment', 'ask-state', 'thread-status', 'thread-notification', 'proposal-decision'],
  replies: ['ask-reply'],
};
export interface OwnPublicationDiagnostic {
  root: OwnRoot;
  kind: string;
  streamId: string;
  submittedSeq: string | null;
  admittedSeq: string;
  leaderReceipt: boolean;
}
export interface OwnPublicationReceipt {
  /** A replayed relay result may no longer retain the original sequence. */
  seq: string | null;
}
const fields = ['root', 'kind', 'streamId', 'submittedSeq', 'admittedSeq', 'leaderReceipt'];
export function publicationSequence(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^(0|[1-9][0-9]{0,19})$/.test(value) &&
    BigInt(value) <= 18446744073709551615n
  );
}
/** A detached, closed diagnostic: never copy a record key, value or generic error. */
export function publicationDiagnostic(value: unknown): OwnPublicationDiagnostic | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return;
  const row = value as Record<string, unknown>;
  if (Object.keys(row).length !== fields.length || fields.some((key) => !Object.hasOwn(row, key)))
    return;
  if (
    typeof row.root !== 'string' ||
    !Object.hasOwn(kinds, row.root) ||
    typeof row.kind !== 'string' ||
    (row.kind !== 'unknown' && !kinds[row.root as OwnRoot].includes(row.kind)) ||
    typeof row.streamId !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(row.streamId) ||
    (row.submittedSeq !== null && !publicationSequence(row.submittedSeq)) ||
    !publicationSequence(row.admittedSeq) ||
    typeof row.leaderReceipt !== 'boolean'
  )
    return;
  return {
    root: row.root as OwnRoot,
    kind: row.kind,
    streamId: row.streamId,
    submittedSeq: row.submittedSeq,
    admittedSeq: row.admittedSeq,
    leaderReceipt: row.leaderReceipt,
  };
}

/** The serve refused an append of this device's own stream with `CONFLICT`: another original
 * already holds that sequence, as when two holders of one link write its shared stream at once.
 * The refused original was never stored, so its sender may drop it and seal again at the stream's
 * new head. Only a refusal the serve answered is this: a lost connection is never. */
export class OwnAppendRefused extends Error {
  constructor(readonly code: string) {
    super(code);
  }
}

/** The serve refused this device's own checkpoint with `CONFLICT`: it already holds a different
 * checkpoint of the stream at that position (an owner fold of it). The refused checkpoint was
 * never stored and no update is lost, so it may be dropped; the connection and the stream are
 * intact. It is answered to the checkpoint alone, so the message stays `CONFLICT` for any caller
 * that reads one. */
export class OwnCheckpointRefused extends Error {
  constructor() {
    super('CONFLICT');
  }
}

/** The stream's leader refused a write because one of its own keys already holds a different value.
 * Own records are immutable: the first value stays, whichever tab wrote it, and nothing of the
 * refused write was appended. A byte-identical write is not a conflict (it is already present).
 * The submitting tab may drop the write and read again; its connection and the stream are intact. */
export class OwnRecordConflict extends Error {
  constructor() {
    super('Own record conflict');
  }
}

export class OwnPublicationTimeout extends Error {
  readonly diagnostic: OwnPublicationDiagnostic | undefined;
  constructor(
    record: OwnRecord,
    streamId: string,
    admittedSeq: string,
    receipt?: OwnPublicationReceipt,
  ) {
    super('Own publication catchup timed out');
    const value = record.value;
    const kind = value && typeof value === 'object' && !Array.isArray(value) ? value.kind : null;
    this.diagnostic = publicationDiagnostic({
      root: record.root,
      kind:
        typeof kind === 'string' &&
        Object.hasOwn(kinds, record.root) &&
        kinds[record.root].includes(kind)
          ? kind
          : 'unknown',
      streamId,
      submittedSeq: receipt?.seq ?? null,
      admittedSeq,
      leaderReceipt: receipt !== undefined,
    });
  }
}
