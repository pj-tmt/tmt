import { uuidIdentifier as identifier, canonicalUuid } from '../contracts/record.js';

export type BoardCategory =
  | { kind: 'general' }
  | { kind: 'repository'; repositoryId: string }
  | { kind: 'room'; roomId: string };

export type BoardActor = { kind: 'owner' } | { kind: 'identity'; identityId: string; name: string };

export interface BoardEntry {
  id: string;
  threadId: string;
  category: BoardCategory;
  author: BoardActor;
  revision: number;
  deleted: boolean;
  createdAtMs: number;
  updatedAtMs: number;
  title?: string;
  body?: string;
}

export interface BoardThreadSummary extends Omit<BoardEntry, 'body'> {
  replyCount: number;
  activitySequence: number;
}

export interface BoardCategoryPage {
  categories: BoardCategory[];
  nextCursor: string | null;
  boardRevision: number;
}

export interface BoardListPage {
  threads: BoardThreadSummary[];
  nextCursor: string | null;
  boardRevision: number;
}

export interface BoardShowPage {
  thread: BoardEntry;
  replies: BoardEntry[];
  nextCursor: string | null;
  boardRevision: number;
}

export interface BoardCreateReceipt {
  entryId: string;
  threadId: string;
  revision: number;
  created: boolean;
  operationId: string;
}

export interface BoardEditReceipt {
  entryId: string;
  revision: number;
  changed: boolean;
  operationId: string;
}

export interface BoardDeleteReceipt {
  entryId: string;
  revision: number;
  deleted: true;
  changed: boolean;
  moderated: boolean;
  operationId: string;
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error(`Invalid ${label}.`);
  return value as Record<string, unknown>;
}

function exactKeys(value: Record<string, unknown>, keys: string[], label: string): void {
  if (Object.keys(value).sort().join(',') !== [...keys].sort().join(','))
    throw new Error(`Invalid ${label}.`);
}

function safeInteger(value: unknown, minimum = 0): number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum)
    throw new Error('Invalid number.');
  return value as number;
}

function text(value: unknown, allowEmpty = false): string {
  if (typeof value !== 'string' || (!allowEmpty && value.length === 0))
    throw new Error('Invalid text.');
  return value;
}

function boundedText(value: unknown, maximumBytes: number, body = false): string {
  const result = text(value);
  if (
    new TextEncoder().encode(result).length > maximumBytes ||
    [...result].some((character) => {
      const code = character.codePointAt(0)!;
      const control = code <= 0x1f || (code >= 0x7f && code <= 0x9f);
      return control && !(body && (character === '\n' || character === '\t'));
    })
  )
    throw new Error('Invalid text.');
  return result;
}

export function boardCategoryKey(category: BoardCategory): string {
  switch (category.kind) {
    case 'general':
      return 'general';
    case 'repository':
      return `repository:${category.repositoryId}`;
    case 'room':
      return `room:${category.roomId}`;
  }
}

function sameCategory(left: BoardCategory, right: BoardCategory): boolean {
  return boardCategoryKey(left) === boardCategoryKey(right);
}

function cursor(value: unknown): string | null {
  if (value === null) return null;
  if (typeof value !== 'string' || value.length === 0 || value.length > 4096)
    throw new Error('Invalid cursor.');
  return value;
}

export function decodeBoardCategory(value: unknown): BoardCategory {
  const item = record(value, 'category');
  if (item.kind === 'general') {
    exactKeys(item, ['kind'], 'category');
    return { kind: 'general' };
  }
  if (item.kind === 'room') {
    exactKeys(item, ['kind', 'roomId'], 'category');
    return { kind: 'room', roomId: canonicalUuid(item.roomId) };
  }
  exactKeys(item, ['kind', 'repositoryId'], 'category');
  if (item.kind !== 'repository') throw new Error('Invalid category.');
  return { kind: 'repository', repositoryId: boundedText(item.repositoryId, 2048) };
}

function decodeActor(value: unknown): BoardActor {
  const item = record(value, 'author');
  if (item.kind === 'owner') {
    exactKeys(item, ['kind'], 'author');
    return { kind: 'owner' };
  }
  exactKeys(item, ['kind', 'identityId', 'name'], 'author');
  if (item.kind !== 'identity') throw new Error('Invalid author.');
  return {
    kind: 'identity',
    identityId: identifier(item.identityId),
    name: boundedText(item.name, 256),
  };
}

export function decodeBoardEntry(value: unknown): BoardEntry {
  const item = record(value, 'board entry');
  const deleted = item.deleted;
  if (typeof deleted !== 'boolean') throw new Error('Invalid board entry.');
  const root = item.id === item.threadId;
  const optional = deleted ? [] : root ? ['title', 'body'] : ['body'];
  exactKeys(
    item,
    [
      'id',
      'threadId',
      'category',
      'author',
      'revision',
      'deleted',
      'createdAtMs',
      'updatedAtMs',
      ...optional,
    ],
    'board entry'
  );
  const entry: BoardEntry = {
    id: identifier(item.id),
    threadId: identifier(item.threadId),
    category: decodeBoardCategory(item.category),
    author: decodeActor(item.author),
    revision: safeInteger(item.revision, 1),
    deleted,
    createdAtMs: safeInteger(item.createdAtMs),
    updatedAtMs: safeInteger(item.updatedAtMs),
  };
  if (!deleted) {
    entry.body = boundedText(item.body, root ? 16_384 : 8_192, true);
    if (root) entry.title = boundedText(item.title, 160);
  }
  return entry;
}

function decodeThread(value: unknown): BoardThreadSummary {
  const item = record(value, 'board thread');
  const deleted = item.deleted;
  if (typeof deleted !== 'boolean') throw new Error('Invalid board thread.');
  exactKeys(
    item,
    [
      'id',
      'threadId',
      'category',
      'author',
      'revision',
      'deleted',
      'createdAtMs',
      'updatedAtMs',
      ...(deleted ? [] : ['title']),
      'replyCount',
      'activitySequence',
    ],
    'board thread'
  );
  const id = identifier(item.id);
  const threadId = identifier(item.threadId);
  if (id !== threadId) throw new Error('Invalid board thread.');
  return {
    id,
    threadId,
    category: decodeBoardCategory(item.category),
    author: decodeActor(item.author),
    revision: safeInteger(item.revision, 1),
    deleted,
    createdAtMs: safeInteger(item.createdAtMs),
    updatedAtMs: safeInteger(item.updatedAtMs),
    ...(deleted ? {} : { title: boundedText(item.title, 160) }),
    replyCount: safeInteger(item.replyCount),
    activitySequence: safeInteger(item.activitySequence, 1),
  };
}

export function decodeCategoryPage(value: unknown): BoardCategoryPage {
  const page = record(value, 'category page');
  exactKeys(page, ['categories', 'nextCursor', 'boardRevision'], 'category page');
  if (!Array.isArray(page.categories)) throw new Error('Invalid category page.');
  return {
    categories: page.categories.map(decodeBoardCategory),
    nextCursor: cursor(page.nextCursor),
    boardRevision: safeInteger(page.boardRevision),
  };
}

export function decodeBoardList(value: unknown): BoardListPage {
  const page = record(value, 'board list');
  exactKeys(page, ['threads', 'nextCursor', 'boardRevision'], 'board list');
  if (!Array.isArray(page.threads)) throw new Error('Invalid board list.');
  return {
    threads: page.threads.map(decodeThread),
    nextCursor: cursor(page.nextCursor),
    boardRevision: safeInteger(page.boardRevision),
  };
}

export function decodeBoardShow(value: unknown): BoardShowPage {
  const page = record(value, 'board thread');
  exactKeys(page, ['thread', 'replies', 'nextCursor', 'boardRevision'], 'board thread');
  if (!Array.isArray(page.replies)) throw new Error('Invalid board thread.');
  const thread = decodeBoardEntry(page.thread);
  const replies = page.replies.map(decodeBoardEntry);
  if (
    thread.id !== thread.threadId ||
    replies.some(
      (reply) =>
        reply.threadId !== thread.id ||
        reply.id === reply.threadId ||
        !sameCategory(reply.category, thread.category)
    )
  )
    throw new Error('Invalid board thread.');
  return {
    thread,
    replies,
    nextCursor: cursor(page.nextCursor),
    boardRevision: safeInteger(page.boardRevision),
  };
}

export function decodeCreateReceipt(value: unknown): BoardCreateReceipt {
  const receipt = record(value, 'board receipt');
  exactKeys(
    receipt,
    ['entryId', 'threadId', 'revision', 'created', 'operationId'],
    'board receipt'
  );
  if (receipt.created !== true) throw new Error('Invalid board receipt.');
  return {
    entryId: identifier(receipt.entryId),
    threadId: identifier(receipt.threadId),
    revision: safeInteger(receipt.revision, 1),
    created: receipt.created,
    operationId: identifier(receipt.operationId),
  };
}

export function decodePostReceipt(value: unknown): BoardCreateReceipt {
  const receipt = decodeCreateReceipt(value);
  if (receipt.entryId !== receipt.threadId) throw new Error('Invalid board receipt.');
  return receipt;
}

export function decodeReplyReceipt(value: unknown): BoardCreateReceipt {
  const receipt = decodeCreateReceipt(value);
  if (receipt.entryId === receipt.threadId) throw new Error('Invalid board receipt.');
  return receipt;
}

export function decodeEditReceipt(value: unknown): BoardEditReceipt {
  const receipt = record(value, 'board receipt');
  exactKeys(receipt, ['entryId', 'revision', 'changed', 'operationId'], 'board receipt');
  if (typeof receipt.changed !== 'boolean') throw new Error('Invalid board receipt.');
  return {
    entryId: identifier(receipt.entryId),
    revision: safeInteger(receipt.revision, 1),
    changed: receipt.changed,
    operationId: identifier(receipt.operationId),
  };
}

export function decodeDeleteReceipt(value: unknown): BoardDeleteReceipt {
  const receipt = record(value, 'board receipt');
  exactKeys(
    receipt,
    ['entryId', 'revision', 'deleted', 'changed', 'moderated', 'operationId'],
    'board receipt'
  );
  if (
    receipt.deleted !== true ||
    typeof receipt.changed !== 'boolean' ||
    typeof receipt.moderated !== 'boolean'
  )
    throw new Error('Invalid board receipt.');
  return {
    entryId: identifier(receipt.entryId),
    revision: safeInteger(receipt.revision, 1),
    deleted: true,
    changed: receipt.changed,
    moderated: receipt.moderated,
    operationId: identifier(receipt.operationId),
  };
}
