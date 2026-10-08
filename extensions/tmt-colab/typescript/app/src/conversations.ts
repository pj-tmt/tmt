import { requireValue } from '@tmt/colab-client';
import { readAskRecords } from './ask-records.js';
import type { OwnState } from './fold-protocol.js';
import { readThreads } from './thread-records.js';
import type { ThreadNotificationRecord, ThreadStatusView } from './thread-status.js';

/** The authorized discussion view of one captured page snapshot: verified threads,
 * comments and Ask conversations, as plain data. Native `export/conversations.rs`
 * builds the same bytes; `vectors/conversations-v1.json` pins both. */
export const CONVERSATIONS_FORMAT = 'tmt-colab-conversations';

export interface ConversationComment {
  writer: string;
  id: string;
  revision: string;
  deleted: boolean;
  body: string;
  deviceName: string;
  at: string;
}
export interface ConversationThread {
  writer: string;
  id: string;
  revision: string;
  anchor: { exact: string; prefix: string; suffix: string } | null;
  resolved: boolean;
  deleted: boolean;
  deviceName: string;
  at: string;
  comments: ConversationComment[];
  status?: ThreadStatusView;
  notifications?: ThreadNotificationRecord[];
}
export interface ConversationAsk {
  writer: string;
  operationId: string;
  deviceName: string;
  agentName: string;
  agent: string;
  machine: string;
  thread: string;
  messageIds: string[];
  issuedAt: number;
  expiresAt: number;
  message: string;
  state: string;
  reason: string | null;
  requestId: string | null;
  reply: { requestId: string; agentId: string; body: string } | null;
}
export interface Conversations {
  format: typeof CONVERSATIONS_FORMAT;
  version: 1;
  spaceId: string;
  pageId: string;
  title: string;
  epoch: string;
  membershipHead: { revision: string; statementHash: string };
  threads: ConversationThread[];
  asks: ConversationAsk[];
}
export interface ConversationsInput {
  spaceId: string;
  pageId: string;
  title: string;
  epoch: string;
  membershipHead: { revision: string; statementHash: string };
  own: OwnState;
  /** Historical signing key per writer, taken from cut-admitted envelopes. */
  signingKey: (writer: string) => Uint8Array | undefined;
  /** Whether a writer may resolve threads: owner-member provenance, native `status_writers`. */
  statusWriter: (writer: string) => boolean;
}

// Identifiers are ASCII, so UTF-16 order is byte order; never a locale comparison.
const byOrder = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
const ref = (writer: string, id: string) => `${writer}:${id}`;

function captureNotification(value: ThreadNotificationRecord): ThreadNotificationRecord {
  return {
    version: value.version,
    kind: value.kind,
    spaceId: value.spaceId,
    pageId: value.pageId,
    epoch: value.epoch,
    senderDevice: value.senderDevice,
    revision: value.revision,
    deleted: value.deleted,
    deviceName: value.deviceName,
    at: value.at,
    operationId: value.operationId,
    status: { writer: value.status.writer, id: value.status.id },
    reason: value.reason,
  };
}
// Native status::Action serializes this declared order; never inherit map insertion order.
function captureStatus(status: ThreadStatusView): ThreadStatusView {
  return {
    version: status.version,
    kind: status.kind,
    spaceId: status.spaceId,
    pageId: status.pageId,
    epoch: status.epoch,
    senderDevice: status.senderDevice,
    revision: status.revision,
    deleted: status.deleted,
    deviceName: status.deviceName,
    at: status.at,
    actionId: status.actionId,
    thread: { writer: status.thread.writer, id: status.thread.id },
    previous: status.previous ? { writer: status.previous.writer, id: status.previous.id } : null,
    resolved: status.resolved,
    actor: status.actor,
    agentName: status.agentName,
    recipients: status.recipients.map((recipient) => ({
      machine: recipient.machine,
      agent: recipient.agent,
      agentName: recipient.agentName,
      operationId: recipient.operationId,
    })),
    ref: { writer: status.ref.writer, id: status.ref.id },
    depth: status.depth,
  };
}

/** Reads only the admitted per-writer projections; a claimed writer in a body never
 * selects another stream. Everything is copied before it is returned. */
export async function projectConversations(input: ConversationsInput): Promise<Conversations> {
  const { spaceId, pageId, epoch } = input;
  const threads = readThreads(
    input.own,
    { spaceId, pageId, epoch },
    input.signingKey,
    input.statusWriter,
  )
    .map((thread): ConversationThread => ({
      writer: thread.ref.writer,
      id: thread.ref.id,
      revision: thread.revision,
      anchor: thread.anchor ? { ...thread.anchor } : null,
      resolved: thread.resolved,
      deleted: thread.deleted,
      deviceName: thread.deviceName,
      at: thread.at,
      comments: thread.comments
        .map((comment) => ({
          writer: comment.ref.writer,
          id: comment.ref.id,
          revision: comment.revision,
          deleted: comment.deleted,
          body: comment.body,
          deviceName: comment.deviceName,
          at: comment.at,
        }))
        .sort((a, b) => byOrder(ref(a.writer, a.id), ref(b.writer, b.id))),
      ...(thread.status ? { status: captureStatus(thread.status) } : {}),
      ...(thread.notifications?.some(
        (value) =>
          thread.status &&
          ref(value.status.writer, value.status.id) ===
            ref(thread.status.ref.writer, thread.status.ref.id),
      )
        ? {
            notifications: thread.notifications
              .filter(
                (value) =>
                  thread.status &&
                  ref(value.status.writer, value.status.id) ===
                    ref(thread.status.ref.writer, thread.status.ref.id),
              )
              .map(captureNotification)
              .sort((a, b) => byOrder(a.operationId, b.operationId)),
          }
        : {}),
    }))
    .sort((a, b) => byOrder(ref(a.writer, a.id), ref(b.writer, b.id)));
  const asks = (await readAskRecords(input.own, { space: spaceId, page: pageId }, input.signingKey))
    .map((view): ConversationAsk => ({
      writer: view.writer,
      operationId: view.intent.operationId,
      deviceName: view.deviceName,
      agentName: view.agentName,
      agent: view.intent.agent,
      machine: view.intent.machine,
      thread: view.intent.thread,
      messageIds: [...view.intent.messageIds],
      issuedAt: view.intent.issuedAt,
      expiresAt: view.intent.expiresAt,
      message: view.intent.message,
      state: view.state,
      reason: view.reason,
      requestId: view.requestId,
      reply: view.reply
        ? {
            requestId: view.reply.requestId,
            agentId: view.reply.agentId,
            body: view.reply.body,
          }
        : null,
    }))
    .sort((a, b) => byOrder(ref(a.writer, a.operationId), ref(b.writer, b.operationId)));
  return {
    format: CONVERSATIONS_FORMAT,
    version: 1,
    spaceId,
    pageId,
    title: input.title,
    epoch,
    // Built field by field: the wire order never depends on the caller's key order.
    membershipHead: {
      revision: input.membershipHead.revision,
      statementHash: input.membershipHead.statementHash,
    },
    threads,
    asks,
  };
}

/** Compact JSON in the declared key order (the object above is built in that order). */
export function serializeConversations(conversations: Conversations): string {
  return JSON.stringify(conversations);
}

// Display escapes: controls, bidi/format marks and line separators become \u{hex}.
function escapable(point: number, keepLayout: boolean): boolean {
  if (keepLayout && (point === 0x0a || point === 0x09)) return false;
  return (
    point < 0x20 ||
    (point >= 0x7f && point <= 0x9f) ||
    point === 0x2028 ||
    point === 0x2029 ||
    point === 0x200e ||
    point === 0x200f ||
    (point >= 0x202a && point <= 0x202e) ||
    (point >= 0x2066 && point <= 0x2069) ||
    point === 0xfeff
  );
}
function display(value: string, keepLayout: boolean): string {
  let out = '';
  for (const char of value) {
    const point = char.codePointAt(0) as number;
    out += escapable(point, keepLayout) ? `\\u{${point.toString(16)}}` : char;
  }
  return out;
}
function longestRun(value: string): number {
  let longest = 0;
  for (const run of value.match(/`+/g) ?? []) longest = Math.max(longest, run.length);
  return longest;
}
/** An inline code span that no label can close or extend. */
function codeSpan(value: string): string {
  const shown = display(value, false);
  if (shown === '') return '(none)';
  const ticks = '`'.repeat(longestRun(shown) + 1);
  const pad = /^[` ]|[` ]$/.test(shown) ? ' ' : '';
  return `${ticks}${pad}${shown}${pad}${ticks}`;
}
/** A fenced block longer than any backtick run inside, so a body cannot end it. */
function fence(value: string): string {
  const shown = display(value, true);
  const ticks = '`'.repeat(Math.max(3, longestRun(shown) + 1));
  return `${ticks}\n${shown}\n${ticks}`;
}
const YEAR_10000_MS = 253402300800000;
function time(value: number | string): string {
  const ms = Number(value);
  requireValue(Number.isSafeInteger(ms) && ms >= 0);
  if (ms >= YEAR_10000_MS) return `unix ms ${ms}`;
  const iso = new Date(ms).toISOString();
  return `${iso.slice(0, 10)} ${iso.slice(11, 19)} UTC`;
}
const chronological =
  <T>(at: (item: T) => number, id: (item: T) => string) =>
  (a: T, b: T) =>
    at(a) - at(b) || byOrder(id(a), id(b));

/** A plain-text reading of the same frozen data. Names and times are labels. */
export function renderConversationsMarkdown(conversations: Conversations): string {
  const lines: string[] = [
    '# Conversations',
    '',
    `- Page: ${codeSpan(conversations.title)}`,
    `- Page ID: ${conversations.pageId}`,
    `- Space: ${conversations.spaceId}`,
    `- Epoch: ${conversations.epoch}`,
    `- Membership head: revision ${conversations.membershipHead.revision}, ${conversations.membershipHead.statementHash}`,
    '',
    'Names and times are labels asserted by each writer. They are not verified identities or clocks.',
    '',
    `## Threads (${conversations.threads.length})`,
    '',
  ];
  if (conversations.threads.length === 0) lines.push('No threads.', '');
  const threads = [...conversations.threads].sort(
    chronological(
      (thread) => Number(thread.at),
      (thread) => ref(thread.writer, thread.id),
    ),
  );
  threads.forEach((thread, index) => {
    lines.push(
      `### Thread ${index + 1}`,
      '',
      `- Thread ID: ${ref(thread.writer, thread.id)}`,
      `- Status: ${thread.deleted ? 'deleted' : thread.resolved ? 'resolved' : 'open'}`,
      `- Started by: ${codeSpan(thread.deviceName)} at ${time(thread.at)}`,
      '',
    );
    if (thread.status) {
      const status = thread.status;
      const actor = status.actor === 'agent' ? (status.agentName ?? 'Agent') : status.deviceName;
      lines.push(
        `- ${status.resolved ? 'Resolved' : 'Reopened'} by: ${codeSpan(actor)} at ${time(status.at)}`,
        '',
      );
      for (const notification of thread.notifications ?? [])
        lines.push(`- Notification ${notification.operationId}: ${notification.reason}`, '');
    }
    if (thread.anchor) lines.push('Quoted text:', '', fence(thread.anchor.exact), '');
    else lines.push('Quoted text: none', '');
    const comments = [...thread.comments].sort(
      chronological(
        (comment) => Number(comment.at),
        (comment) => ref(comment.writer, comment.id),
      ),
    );
    comments.forEach((comment, position) => {
      lines.push(
        `#### Comment ${position + 1}`,
        '',
        `- Comment ID: ${ref(comment.writer, comment.id)}`,
        `- By: ${codeSpan(comment.deviceName)} at ${time(comment.at)} (writer ${comment.writer})`,
        `- Revision: ${comment.revision}`,
        '',
      );
      lines.push(comment.deleted ? 'Deleted.' : fence(comment.body), '');
    });
  });
  lines.push(`## Asks (${conversations.asks.length})`, '');
  if (conversations.asks.length === 0) lines.push('No asks.', '');
  const asks = [...conversations.asks].sort(
    chronological(
      (ask) => ask.issuedAt,
      (ask) => ref(ask.writer, ask.operationId),
    ),
  );
  asks.forEach((ask, index) => {
    lines.push(
      `### Ask ${index + 1}`,
      '',
      `- Operation ID: ${ask.operationId}`,
      `- Asked by: ${codeSpan(ask.deviceName)} at ${time(ask.issuedAt)} (writer ${ask.writer})`,
      `- Agent: ${codeSpan(ask.agentName)} (${ask.agent}) on machine ${ask.machine}`,
      `- Thread: ${ask.thread}`,
      `- State: ${ask.state}${ask.reason ? ` (${ask.reason})` : ''}`,
      `- Request ID: ${ask.requestId ?? 'none'}`,
      '',
      'Message sent:',
      '',
      fence(ask.message),
      '',
    );
    if (ask.reply) lines.push('Reply:', '', fence(ask.reply.body), '');
    else lines.push('No reply recorded.', '');
  });
  return lines.join('\n');
}
