import { describe, expect, it } from 'vitest';
import {
  decodeBoardList,
  decodeBoardShow,
  decodeCategoryPage,
  decodeCreateReceipt,
  decodeDeleteReceipt,
  decodePostReceipt,
  decodeReplyReceipt,
  decodeBoardCategory,
  boardCategoryKey,
} from './board-contract.js';

const threadId = '11111111-1111-4111-8111-111111111111';
const replyId = '22222222-2222-4222-8222-222222222222';
const operationId = '33333333-3333-4333-8333-333333333333';
const category = { kind: 'general' } as const;
const author = { kind: 'owner' } as const;

describe('local board contract', () => {
  it('preserves room category authority and rejects mixed or invalid scopes', () => {
    const roomId = '11111111-1111-7111-8111-111111111111';
    const room = { kind: 'room', roomId } as const;
    expect(decodeBoardCategory(room)).toEqual(room);
    expect(boardCategoryKey(room)).toBe(`room:${roomId}`);
    expect(boardCategoryKey(room)).not.toBe(
      boardCategoryKey({ kind: 'repository', repositoryId: roomId })
    );
    for (const invalid of [
      { kind: 'room' },
      { kind: 'room', roomId: null },
      { kind: 'room', roomId: 'Design' },
      { kind: 'room', roomId: '00000000-0000-0000-0000-000000000000' },
      { ...room, repositoryId: 'example.com/project' },
      { kind: 'general', roomId },
    ])
      expect(() => decodeBoardCategory(invalid)).toThrow();
  });
  it('decodes bounded category, summary, entry and receipt projections', () => {
    expect(
      decodeCategoryPage({
        categories: [category, { kind: 'repository', repositoryId: 'github.com/acme/repo' }],
        nextCursor: null,
        boardRevision: 2,
      })
    ).toMatchObject({ boardRevision: 2 });
    expect(
      decodeBoardList({
        threads: [
          {
            id: threadId,
            threadId,
            category,
            author,
            revision: 1,
            deleted: false,
            createdAtMs: 1,
            updatedAtMs: 2,
            title: 'Status',
            replyCount: 1,
            activitySequence: 2,
          },
        ],
        nextCursor: 'cursor',
        boardRevision: 2,
      }).threads[0]
    ).toMatchObject({ id: threadId, title: 'Status', replyCount: 1 });
    const tombstone = decodeBoardShow({
      thread: {
        id: threadId,
        threadId,
        category,
        author,
        revision: 1,
        deleted: false,
        createdAtMs: 1,
        updatedAtMs: 1,
        title: 'Status',
        body: '<script>plain text</script>',
      },
      replies: [
        {
          id: replyId,
          threadId,
          category,
          author,
          revision: 2,
          deleted: true,
          createdAtMs: 2,
          updatedAtMs: 3,
        },
      ],
      nextCursor: null,
      boardRevision: 3,
    }).replies[0];
    expect(tombstone).toMatchObject({ id: replyId, deleted: true });
    expect(tombstone).not.toHaveProperty('title');
    expect(tombstone).not.toHaveProperty('body');
    expect(
      decodeCreateReceipt({ entryId: threadId, threadId, revision: 1, created: true, operationId })
    ).toMatchObject({ created: true, operationId });
    expect(() =>
      decodePostReceipt({
        entryId: replyId,
        threadId,
        revision: 1,
        created: true,
        operationId,
      })
    ).toThrow('Invalid board receipt');
    expect(() =>
      decodeReplyReceipt({
        entryId: threadId,
        threadId,
        revision: 1,
        created: true,
        operationId,
      })
    ).toThrow('Invalid board receipt');
    expect(
      decodeDeleteReceipt({
        entryId: replyId,
        revision: 2,
        deleted: true,
        changed: true,
        moderated: true,
        operationId,
      })
    ).toMatchObject({ deleted: true, moderated: true });
  });

  it('rejects extra fields, executable-shaped content fields on tombstones and invalid IDs', () => {
    expect(() =>
      decodeCategoryPage({ categories: [category], nextCursor: null, boardRevision: 0, token: 'x' })
    ).toThrow('Invalid category page');
    expect(() =>
      decodeBoardShow({
        thread: {
          id: threadId,
          threadId,
          category,
          author,
          revision: 2,
          deleted: true,
          createdAtMs: 1,
          updatedAtMs: 2,
          body: 'must not survive deletion',
        },
        replies: [],
        nextCursor: null,
        boardRevision: 2,
      })
    ).toThrow('Invalid board entry');
    expect(() =>
      decodeCreateReceipt({
        entryId: 'not-a-uuid',
        threadId,
        revision: 1,
        created: true,
        operationId,
      })
    ).toThrow('Invalid identifier');
    expect(() =>
      decodeCreateReceipt({
        entryId: threadId,
        threadId,
        revision: 1,
        created: false,
        operationId,
      })
    ).toThrow('Invalid board receipt');
    expect(() =>
      decodeBoardShow({
        thread: {
          id: threadId,
          threadId,
          category,
          author,
          revision: 1,
          deleted: false,
          createdAtMs: 1,
          updatedAtMs: 1,
          title: 'Status',
          body: 'Body',
        },
        replies: [
          {
            id: replyId,
            threadId,
            category: { kind: 'repository', repositoryId: 'github.com/acme/repo' },
            author,
            revision: 1,
            deleted: false,
            createdAtMs: 2,
            updatedAtMs: 2,
            body: 'Wrong category',
          },
        ],
        nextCursor: null,
        boardRevision: 2,
      })
    ).toThrow('Invalid board thread');
    expect(() =>
      decodeBoardList({
        threads: [
          {
            id: threadId,
            threadId,
            category,
            author,
            revision: 1,
            deleted: false,
            createdAtMs: 1,
            updatedAtMs: 1,
            title: 'invalid\nheading',
            replyCount: 0,
            activitySequence: 1,
          },
        ],
        nextCursor: null,
        boardRevision: 1,
      })
    ).toThrow('Invalid text');
  });
});
