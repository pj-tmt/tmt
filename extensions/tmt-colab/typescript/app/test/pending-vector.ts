/** Admitted projection fixtures shared with the native summary; no fixture body proves a signature. */
import type { PageView } from '../src/transport.js';
import type { Proposal, QuoteSelector, ThreadView } from '../src/thread-records.js';

export interface PendingVector {
  name: string;
  source: string;
  deviceWriters: string[];
  threads: {
    writer: string;
    id: string;
    anchor: QuoteSelector | null;
    deleted?: boolean;
    resolved?: boolean;
    decided?: boolean;
    proposal?: Proposal;
    status?: { actor: 'person' | 'agent'; resolved: boolean };
    comments: { writer: string; id: string; body: string; sequence: string; deleted: boolean }[];
  }[];
  asks: { writer: string; thread: string; messageIds: string[]; replied: boolean }[];
  expected: { kind: string; id: string }[];
}

export function pendingVectorData(
  vector: PendingVector,
): Pick<PageView, 'source' | 'threads' | 'asks'> {
  const stamp = {
    version: 1 as const,
    spaceId: 'fixture',
    pageId: '10000000-0000-4000-8000-000000000001',
    epoch: '1',
    revision: '1',
    deleted: false,
    deviceName: 'Fixture',
    at: '1',
  };
  const threads: ThreadView[] = vector.threads.map((row) => {
    const ref = { writer: row.writer, id: row.id };
    return {
      ...stamp,
      senderDevice: row.writer,
      kind: 'thread',
      threadId: row.id,
      ref,
      createdAt: '1',
      anchor: row.anchor,
      resolved: row.resolved ?? false,
      deleted: row.deleted ?? false,
      proposal: row.proposal,
      comments: row.comments.map((comment) => ({
        ...stamp,
        senderDevice: comment.writer,
        kind: 'comment',
        messageId: comment.id,
        ref: { writer: comment.writer, id: comment.id },
        thread: ref,
        body: comment.body,
        sequence: comment.sequence,
        deleted: comment.deleted,
        deviceAuthor: vector.deviceWriters.includes(comment.writer),
      })),
      ...(row.status
        ? {
            status: {
              ...stamp,
              senderDevice: row.writer,
              kind: 'thread-status',
              actionId: row.id,
              thread: ref,
              previous: null,
              resolved: row.status.resolved,
              actor: row.status.actor,
              agentName: null,
              recipients: [],
              ref,
              depth: 1,
            },
          }
        : {}),
      ...(row.decided !== undefined
        ? {
            decision: {
              ...stamp,
              senderDevice: row.writer,
              kind: 'proposal-decision',
              actionId: row.id,
              thread: ref,
              previous: null,
              decision: 'approved',
              deleted: !row.decided,
            },
          }
        : {}),
    };
  });
  return {
    source: vector.source,
    threads,
    asks: vector.asks.map((row, index) => ({
      ...row,
      operationId: String(index),
      message: 'Fixture',
      agent: row.writer,
      agentName: 'Agent 1',
      deviceName: 'Fixture',
      issuedAt: 0,
      machine: row.writer,
      state: 'accepted',
      canTrack: false,
      ...(row.replied ? { reply: 'Reply' } : {}),
    })),
  };
}
