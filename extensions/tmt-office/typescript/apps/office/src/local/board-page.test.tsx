import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { LocalRuntime } from './local-runtime.js';
import { LocalHttpError, LocalRuntimeContext } from './local-runtime.js';
import { LocalBoardPage } from './board-page.js';
import type { BoardCategory } from './board-contract.js';

const threadId = '11111111-1111-4111-8111-111111111111';
const replyId = '22222222-2222-4222-8222-222222222222';
const operationId = '33333333-3333-4333-8333-333333333333';
const category = { kind: 'general' } as const;
const owner = { kind: 'owner' } as const;
const identity = {
  kind: 'identity',
  identityId: '44444444-4444-4444-8444-444444444444',
  name: 'Alice',
} as const;
const thread = {
  id: threadId,
  threadId,
  category,
  author: owner,
  revision: 1,
  deleted: false,
  createdAtMs: 1,
  updatedAtMs: 2,
  title: 'Current work',
  body: '<script>plain text</script>',
};
const reply = {
  id: replyId,
  threadId,
  category,
  author: identity,
  revision: 1,
  deleted: false,
  createdAtMs: 2,
  updatedAtMs: 2,
  body: 'Question from Alice',
};

function runtime(): LocalRuntime {
  const { body: _body, ...threadSummary } = thread;
  return {
    world: { show: vi.fn(), save: vi.fn() },
    whiteboards: { show: vi.fn(), save: vi.fn() },
    notebooks: { read: vi.fn() },
    propCatalog: { list: vi.fn(), load: vi.fn(), install: vi.fn() },
    whiteboardSnapshots: { capture: vi.fn(), show: vi.fn(), attachImage: vi.fn(), image: vi.fn() },
    dispatch: { send: vi.fn() },
    requests: { list: vi.fn(), show: vi.fn(), receipt: vi.fn() },
    rooms: { list: vi.fn(async () => []), save: vi.fn(), retire: vi.fn() },
    profiles: {
      list: async () => [],
      show: async () => {
        throw new Error('Profile not used by board tests.');
      },
      apply: async () => {
        throw new Error('Profile not used by board tests.');
      },
    },
    avatars: { list: async () => ({ catalogRevision: 0, packs: [] }) },
    resolveProps: async () => [],
    preview: async () => {
      throw new Error('Preview not used by board tests.');
    },
    avatarPreview: async () => {
      throw new Error('Avatar preview not used by board tests.');
    },
    dispose: () => undefined,
    board: {
      categories: vi.fn(async () => ({
        categories: [category],
        nextCursor: null,
        boardRevision: 2,
      })),
      list: vi.fn(async () => ({
        threads: [
          {
            ...threadSummary,
            replyCount: 1,
            activitySequence: 2,
          },
        ],
        nextCursor: null,
        boardRevision: 2,
      })),
      show: vi.fn(async () => ({ thread, replies: [reply], nextCursor: null, boardRevision: 2 })),
      post: vi.fn(async () => ({
        entryId: threadId,
        threadId,
        revision: 1,
        created: true,
        operationId,
      })),
      reply: vi.fn(async () => ({
        entryId: replyId,
        threadId,
        revision: 1,
        created: true,
        operationId,
      })),
      edit: vi.fn(async () => ({ entryId: threadId, revision: 2, changed: true, operationId })),
      delete: vi.fn(async () => ({
        entryId: replyId,
        revision: 2,
        deleted: true as const,
        changed: true,
        moderated: true,
        operationId,
      })),
    },
  };
}

function mount(active: LocalRuntime, entryCategory?: BoardCategory) {
  const content = (entryCategory?: BoardCategory) => (
    <LocalRuntimeContext value={active}>
      <LocalBoardPage entryCategory={entryCategory} />
    </LocalRuntimeContext>
  );
  const view = render(content(entryCategory));
  return { ...view, openCategory: (next: BoardCategory) => view.rerender(content(next)) };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, decline) => {
    resolve = accept;
    reject = decline;
  });
  return { promise, resolve, reject };
}

describe('local Office board', () => {
  beforeEach(() => {
    vi.spyOn(globalThis.crypto, 'randomUUID').mockReturnValue(operationId);
  });

  it('protects a room draft across spatial entries, then explicitly discards before retargeting', async () => {
    const active = runtime();
    const room = { kind: 'room', roomId: replyId } as const;
    vi.mocked(active.rooms.list).mockResolvedValue([
      { id: replyId, name: 'Design', revision: 1, retired: false, memberIds: [] },
    ]);
    vi.mocked(active.board.list).mockResolvedValue({
      threads: [],
      nextCursor: null,
      boardRevision: 0,
    });
    const view = mount(active, room);
    await screen.findByRole('button', { name: 'Room · Design' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'Room draft');
    await userEvent.type(screen.getByLabelText('Message'), 'Only for Design');
    view.openCategory({ ...room });
    expect(screen.queryByRole('region', { name: 'Switch discussion confirmation' })).toBeNull();
    expect(screen.getByDisplayValue('Room draft')).toBeTruthy();
    view.openCategory(category);
    await screen.findByRole('region', { name: 'Switch discussion confirmation' });
    expect(active.board.list).not.toHaveBeenCalledWith(expect.objectContaining({ category }));
    await userEvent.click(screen.getByRole('button', { name: 'Keep this discussion' }));
    expect(screen.getByDisplayValue('Only for Design')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() =>
      expect(active.board.post).toHaveBeenCalledWith({
        category: room,
        title: 'Room draft',
        body: 'Only for Design',
        operationId,
      })
    );
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'Discard me');
    view.openCategory({ ...category });
    await userEvent.click(await screen.findByRole('button', { name: 'Discard drafts and switch' }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'General' }).getAttribute('aria-pressed')).toBe(
        'true'
      )
    );
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    expect((screen.getByLabelText('Title') as HTMLInputElement).value).toBe('');
    expect(active.board.post).toHaveBeenCalledTimes(1);
  });

  it('blocks a requested switch if a draft becomes in-flight and retains the original retry scope', async () => {
    const active = runtime();
    const room = { kind: 'room', roomId: replyId } as const;
    const pending = deferred<Awaited<ReturnType<LocalRuntime['board']['post']>>>();
    vi.mocked(active.board.list).mockResolvedValue({
      threads: [],
      nextCursor: null,
      boardRevision: 0,
    });
    vi.mocked(active.board.post).mockReturnValueOnce(pending.promise);
    const view = mount(active, room);
    await screen.findByText('No posts in this category yet.');
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'Frozen');
    await userEvent.type(screen.getByLabelText('Message'), 'Original room');
    view.openCategory(category);
    await screen.findByRole('button', { name: 'Discard drafts and switch' });
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    expect(
      (screen.getByRole('button', { name: 'Discard drafts and switch' }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
    expect((screen.getByLabelText('Title') as HTMLInputElement).disabled).toBe(true);
    pending.reject(new TypeError('Response lost'));
    await screen.findByRole('alert');
    expect(
      (screen.getByRole('button', { name: 'Discard drafts and switch' }) as HTMLButtonElement)
        .disabled
    ).toBe(true);
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() => expect(active.board.post).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.post).mock.calls[1]).toEqual(
      vi.mocked(active.board.post).mock.calls[0]
    );
    expect(vi.mocked(active.board.post).mock.calls[1]![0].category).toEqual(room);
    await userEvent.click(screen.getByRole('button', { name: 'Discard drafts and switch' }));
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'General' }).getAttribute('aria-pressed')).toBe(
        'true'
      )
    );
  });

  it('guards reply and edit drafts together before switching threads', async () => {
    const active = runtime();
    const second = { ...thread, id: operationId, threadId: operationId, title: 'Other topic' };
    vi.mocked(active.board.list).mockResolvedValue({
      threads: [thread, second].map(({ body: _body, ...item }) => ({
        ...item,
        replyCount: 0,
        activitySequence: 2,
      })),
      nextCursor: null,
      boardRevision: 2,
    });
    vi.mocked(active.board.show).mockImplementation(async ({ threadId: id }) => ({
      thread: id === operationId ? second : thread,
      replies: [],
      nextCursor: null,
      boardRevision: 2,
    }));
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.type(screen.getByLabelText('Message'), 'Reply draft');
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    await userEvent.type(screen.getByLabelText('Title'), ' edited');
    await userEvent.click(screen.getByRole('button', { name: /Other topic/ }));
    await screen.findByRole('region', { name: 'Switch discussion confirmation' });
    expect(active.board.show).not.toHaveBeenCalledWith(
      expect.objectContaining({ threadId: operationId })
    );
    await userEvent.click(screen.getByRole('button', { name: 'Keep this discussion' }));
    expect(screen.getByDisplayValue('Reply draft')).toBeTruthy();
    expect(screen.getByDisplayValue('Current work edited')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: /Other topic/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Discard drafts and switch' }));
    await screen.findByRole('heading', { name: 'Other topic' });
    expect((screen.getByLabelText('Message') as HTMLTextAreaElement).value).toBe('');
    expect(screen.queryByRole('button', { name: 'Save edit' })).toBeNull();
    expect(active.board.edit).not.toHaveBeenCalled();
    expect(active.board.reply).not.toHaveBeenCalled();
  });

  it('retains uncertain deletion intent and blocks external scope changes across refresh', async () => {
    const active = runtime();
    vi.mocked(active.board.delete).mockRejectedValueOnce(new TypeError('Lost deletion receipt'));
    const view = mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await screen.findByRole('alert');
    vi.mocked(active.board.show).mockResolvedValue({
      thread: { ...thread, revision: 3, deleted: true, title: undefined, body: undefined },
      replies: [reply],
      nextCursor: null,
      boardRevision: 4,
    });
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await screen.findByText(/Office owner · revision 3/);
    expect(screen.getByText('Deleted entry')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Stop retrying deletion' })).toBeTruthy();
    view.openCategory({ kind: 'room', roomId: replyId });
    expect(
      (
        (await screen.findByRole('button', {
          name: 'Discard drafts and switch',
        })) as HTMLButtonElement
      ).disabled
    ).toBe(true);
    await userEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(active.board.delete).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.delete).mock.calls.map(([input]) => input)).toEqual([
      { entryId: threadId, ifRevision: 1, moderate: false, operationId },
      { entryId: threadId, ifRevision: 1, moderate: false, operationId },
    ]);
    await userEvent.click(screen.getByRole('button', { name: 'Discard drafts and switch' }));
    await waitFor(() =>
      expect(active.board.list).toHaveBeenCalledWith(
        expect.objectContaining({ category: { kind: 'room', roomId: replyId } })
      )
    );
  });

  it('does not implicitly switch away from a reply draft when ordering or posting a new thread', async () => {
    const active = runtime();
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.type(screen.getByLabelText('Message'), 'Still replying here');
    const other = { ...thread, id: replyId, threadId: replyId, title: 'Newly active topic' };
    const { body: _body, ...summary } = other;
    vi.mocked(active.board.list).mockResolvedValue({
      threads: [{ ...summary, replyCount: 0, activitySequence: 3 }],
      nextCursor: null,
      boardRevision: 3,
    });
    await userEvent.selectOptions(screen.getByLabelText('Order'), 'recent');
    await waitFor(() =>
      expect(active.board.list).toHaveBeenCalledWith(expect.objectContaining({ view: 'recent' }))
    );
    await screen.findByRole('button', { name: /Newly active topic/ });
    expect(screen.getByDisplayValue('Still replying here')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Current work' })).toBeTruthy();
    vi.mocked(active.board.post).mockResolvedValue({
      entryId: replyId,
      threadId: replyId,
      revision: 1,
      created: true,
      operationId,
    });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    const compose = screen.getByRole('button', { name: 'Post as owner' }).closest('form')!;
    await userEvent.type(within(compose).getByLabelText('Title'), 'Newly active topic');
    await userEvent.type(within(compose).getByLabelText('Message'), 'Independent new post');
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Post as owner' })).toBeNull());
    expect(screen.getByDisplayValue('Still replying here')).toBeTruthy();
    expect(active.board.show).not.toHaveBeenCalledWith(
      expect.objectContaining({ threadId: replyId })
    );
    await userEvent.click(screen.getByRole('button', { name: 'Reply as owner' }));
    await waitFor(() =>
      expect(active.board.reply).toHaveBeenCalledWith({
        threadId,
        body: 'Still replying here',
        operationId,
      })
    );
  });

  it('renders hostile text literally and keeps owner and moderation actions distinct', async () => {
    const active = runtime();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const view = mount(active);
    expect(await screen.findByRole('heading', { name: 'Current work' })).toBeTruthy();
    expect(active.board.list).toHaveBeenCalledWith({ category, view: 'updated', limit: 20 });
    expect(screen.getByText('<script>plain text</script>')).toBeTruthy();
    expect(view.container.querySelector('script')).toBeNull();
    expect(screen.getByRole('button', { name: 'Edit' })).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Moderate delete' }));
    await waitFor(() =>
      expect(active.board.delete).toHaveBeenCalledWith({
        entryId: replyId,
        ifRevision: 1,
        moderate: true,
        operationId,
      })
    );
  });

  it('labels an uncertain draft with its retained destination after changing category', async () => {
    const active = runtime();
    const repository = { kind: 'repository', repositoryId: 'github.com/Example/Project' } as const;
    vi.mocked(active.board.categories).mockResolvedValue({
      categories: [category, repository],
      nextCursor: null,
      boardRevision: 2,
    });
    vi.mocked(active.board.post).mockRejectedValueOnce(new TypeError('Connection lost'));
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'A durable idea');
    await userEvent.type(screen.getAllByLabelText('Message')[0]!, 'Keep this in General');
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await screen.findByRole('alert');
    await userEvent.click(screen.getByRole('button', { name: repository.repositoryId }));
    expect(
      screen.getByText('Posting to General. Sharing here does not dispatch a task.')
    ).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() => expect(active.board.post).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.post).mock.calls[1]).toEqual(
      vi.mocked(active.board.post).mock.calls[0]
    );
  });

  it('posts as the browser owner with one retained operation ID and no actor field', async () => {
    const active = runtime();
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'A discovery');
    await userEvent.type(screen.getAllByLabelText('Message')[0]!, 'Useful details');
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() =>
      expect(active.board.post).toHaveBeenCalledWith({
        category,
        title: 'A discovery',
        body: 'Useful details',
        operationId,
      })
    );
    expect((active.board.post as ReturnType<typeof vi.fn>).mock.calls[0]![0]).not.toHaveProperty(
      'actor'
    );
  });

  it('retains an uncertain post while its composer is closed and reopened', async () => {
    const active = runtime();
    vi.mocked(active.board.post).mockRejectedValueOnce(new TypeError('Lost receipt'));
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'Keep on close');
    const compose = screen.getByRole('button', { name: 'Post as owner' }).closest('form')!;
    await userEvent.type(within(compose).getByLabelText('Message'), 'Original body');
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await screen.findByRole('alert');
    await userEvent.click(screen.getByRole('button', { name: 'Close draft' }));
    expect(screen.queryByDisplayValue('Keep on close')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    expect(screen.getByDisplayValue('Keep on close')).toBeTruthy();
    expect((screen.getByLabelText('Title') as HTMLInputElement).disabled).toBe(true);
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() => expect(active.board.post).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.post).mock.calls[1]).toEqual(
      vi.mocked(active.board.post).mock.calls[0]
    );
  });

  it('keeps an uncertain reply retry reachable when a refresh observes a deleted thread', async () => {
    const active = runtime();
    vi.mocked(active.board.reply).mockRejectedValueOnce(new TypeError('Lost receipt'));
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.type(screen.getByLabelText('Message'), 'Sent before deletion');
    await userEvent.click(screen.getByRole('button', { name: 'Reply as owner' }));
    await screen.findByRole('alert');
    vi.mocked(active.board.show).mockResolvedValue({
      thread: { ...thread, deleted: true, body: undefined, title: undefined, revision: 2 },
      replies: [reply],
      nextCursor: null,
      boardRevision: 4,
    });
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await screen.findByRole('heading', { name: 'Deleted post' });
    expect(screen.getByDisplayValue('Sent before deletion')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Reply as owner' }));
    await waitFor(() => expect(active.board.reply).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.reply).mock.calls[1]).toEqual(
      vi.mocked(active.board.reply).mock.calls[0]
    );
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Reply as owner' })).toBeNull()
    );
  });

  it('retries an unchanged draft with the exact same operation ID', async () => {
    const retryOperationId = '55555555-5555-4555-8555-555555555555';
    vi.mocked(globalThis.crypto.randomUUID)
      .mockReset()
      .mockReturnValueOnce(operationId)
      .mockReturnValue(retryOperationId);
    const active = runtime();
    vi.mocked(active.board.post)
      .mockRejectedValueOnce(new TypeError('temporarily disconnected'))
      .mockResolvedValueOnce({
        entryId: threadId,
        threadId,
        revision: 1,
        created: true,
        operationId,
      });
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'Retry me');
    await userEvent.type(screen.getAllByLabelText('Message')[0]!, 'Same body');
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    expect((await screen.findByRole('alert')).textContent).toContain('draft is still here');
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await waitFor(() => expect(screen.queryByText('Loading the board…')).toBeNull());
    expect(screen.getByDisplayValue('Retry me')).toBeTruthy();
    expect(screen.getByDisplayValue('Same body')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() => expect(active.board.post).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.post).mock.calls.map(([input]) => input.operationId)).toEqual([
      operationId,
      operationId,
    ]);
  });

  it('reuses one committed mutation after an uncertain HTTP 500 response', async () => {
    const active = runtime();
    const committed = new Map<string, unknown>();
    vi.mocked(active.board.post).mockImplementation(async (input) => {
      const existing = committed.get(input.operationId);
      if (!existing) {
        committed.set(input.operationId, input);
        throw new LocalHttpError(500, 'STORAGE_ERROR');
      }
      expect(input).toEqual(existing);
      return {
        entryId: threadId,
        threadId,
        revision: 1,
        created: true,
        operationId: input.operationId,
      };
    });
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getByLabelText('Title'), 'Committed once');
    await userEvent.type(screen.getAllByLabelText('Message')[0]!, 'Retry the receipt');
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    expect((await screen.findByRole('alert')).textContent).toContain('draft is still here');
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await waitFor(() => expect(screen.queryByText('Loading the board…')).toBeNull());
    await userEvent.click(screen.getByRole('button', { name: 'Post as owner' }));
    await waitFor(() => expect(active.board.post).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.post).mock.calls[0]![0]).toEqual(
      vi.mocked(active.board.post).mock.calls[1]![0]
    );
    expect(committed.size).toBe(1);
  });

  it('keeps post, reply, and edit drafts mounted across failed and successful refreshes', async () => {
    const active = runtime();
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'New post' }));
    await userEvent.type(screen.getAllByLabelText('Title')[0]!, 'Post draft');
    await userEvent.type(screen.getAllByLabelText('Message')[0]!, 'Post body draft');
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const editForm = screen.getByRole('button', { name: 'Save edit' }).closest('form')!;
    const editTitle = within(editForm).getByLabelText('Title');
    const editBody = within(editForm).getByLabelText('Message');
    await userEvent.clear(editTitle);
    await userEvent.type(editTitle, 'Edit draft');
    await userEvent.clear(editBody);
    await userEvent.type(editBody, 'Edit body draft');
    const replyForm = screen.getByRole('button', { name: 'Reply as owner' }).closest('form')!;
    await userEvent.type(within(replyForm).getByLabelText('Message'), 'Reply draft');

    vi.mocked(active.board.categories).mockRejectedValueOnce(new TypeError('offline'));
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    expect((await screen.findByRole('alert')).textContent).toContain('draft is still here');
    expect(screen.getByDisplayValue('Post draft')).toBeTruthy();
    expect(screen.getByDisplayValue('Edit draft')).toBeTruthy();
    expect(screen.getByDisplayValue('Reply draft')).toBeTruthy();

    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await waitFor(() => expect(screen.queryByText('Loading the board…')).toBeNull());
    expect(screen.getByDisplayValue('Post body draft')).toBeTruthy();
    expect(screen.getByDisplayValue('Edit body draft')).toBeTruthy();
    expect(screen.getByDisplayValue('Reply draft')).toBeTruthy();
  });

  it('retries a lost edit with its original operation ID and revision after refresh', async () => {
    const active = runtime();
    vi.mocked(active.board.edit)
      .mockRejectedValueOnce(new TypeError('lost response'))
      .mockResolvedValueOnce({ entryId: threadId, revision: 2, changed: true, operationId });
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const form = screen.getByRole('button', { name: 'Save edit' }).closest('form')!;
    await userEvent.clear(within(form).getByLabelText('Title'));
    await userEvent.type(within(form).getByLabelText('Title'), 'Retried edit');
    await userEvent.click(screen.getByRole('button', { name: 'Save edit' }));
    expect((await screen.findByRole('alert')).textContent).toContain('draft is still here');

    vi.mocked(active.board.show).mockResolvedValue({
      thread: { ...thread, revision: 2, title: 'Concurrent title' },
      replies: [reply],
      nextCursor: null,
      boardRevision: 3,
    });
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await waitFor(() => expect(screen.queryByText('Loading the board…')).toBeNull());
    expect(screen.getByDisplayValue('Retried edit')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Save edit' }));
    await waitFor(() => expect(active.board.edit).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.edit).mock.calls.map(([input]) => input)).toEqual([
      expect.objectContaining({ operationId, ifRevision: 1, title: 'Retried edit' }),
      expect.objectContaining({ operationId, ifRevision: 1, title: 'Retried edit' }),
    ]);
  });

  it('retains a selected second-page category after its category refresh', async () => {
    const repository = { kind: 'repository', repositoryId: 'github.com/Org/Repo' } as const;
    const active = runtime();
    vi.mocked(active.board.categories)
      .mockResolvedValueOnce({
        categories: [category],
        nextCursor: 'categories-2',
        boardRevision: 2,
      })
      .mockResolvedValueOnce({ categories: [repository], nextCursor: null, boardRevision: 2 })
      .mockResolvedValueOnce({
        categories: [category],
        nextCursor: 'categories-2',
        boardRevision: 2,
      })
      .mockResolvedValue({ categories: [repository], nextCursor: null, boardRevision: 2 });
    vi.mocked(active.board.list).mockImplementation(async (input) => ({
      threads:
        input.category.kind === 'general'
          ? [{ ...thread, body: undefined, replyCount: 1, activitySequence: 2 }]
          : [],
      nextCursor: null,
      boardRevision: 2,
    }));
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'More categories' }));
    await userEvent.click(await screen.findByRole('button', { name: repository.repositoryId }));
    await waitFor(() =>
      expect(active.board.list).toHaveBeenCalledWith(
        expect.objectContaining({ category: repository })
      )
    );
    expect(
      screen.getByRole('button', { name: repository.repositoryId }).getAttribute('aria-pressed')
    ).toBe('true');
    await userEvent.click(screen.getByRole('button', { name: 'More categories' }));
    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: repository.repositoryId })).toHaveLength(1)
    );
  });

  it('preserves a page-two selection, drafts, and uncertain reply across refresh', async () => {
    const secondId = '66666666-6666-4666-8666-666666666666';
    const secondThread = {
      ...thread,
      id: secondId,
      threadId: secondId,
      title: 'Second page work',
      body: 'Second page body',
    };
    const { body: _firstBody, ...firstSummary } = thread;
    const { body: _secondBody, ...secondSummary } = secondThread;
    const active = runtime();
    vi.mocked(active.board.list).mockImplementation(async (input) =>
      input.cursor
        ? {
            threads: [{ ...secondSummary, replyCount: 0, activitySequence: 3 }],
            nextCursor: null,
            boardRevision: 3,
          }
        : {
            threads: [{ ...firstSummary, replyCount: 1, activitySequence: 2 }],
            nextCursor: 'threads-2',
            boardRevision: 3,
          }
    );
    vi.mocked(active.board.show).mockImplementation(async (input) => ({
      thread: input.threadId === secondId ? secondThread : thread,
      replies: input.threadId === secondId ? [] : [reply],
      nextCursor: null,
      boardRevision: 3,
    }));
    vi.mocked(active.board.reply)
      .mockRejectedValueOnce(new TypeError('lost response'))
      .mockResolvedValueOnce({
        entryId: replyId,
        threadId: secondId,
        revision: 1,
        created: true,
        operationId,
      });
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'More threads' }));
    await userEvent.click(screen.getByRole('button', { name: /Second page work/ }));
    await screen.findByRole('heading', { name: 'Second page work' });
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const editForm = screen.getByRole('button', { name: 'Save edit' }).closest('form')!;
    await userEvent.clear(within(editForm).getByLabelText('Title'));
    await userEvent.type(within(editForm).getByLabelText('Title'), 'Second page draft');
    const replyForm = screen.getByRole('button', { name: 'Reply as owner' }).closest('form')!;
    await userEvent.type(within(replyForm).getByLabelText('Message'), 'Uncertain page-two reply');
    await userEvent.click(screen.getByRole('button', { name: 'Reply as owner' }));
    expect((await screen.findByRole('alert')).textContent).toContain('draft is still here');
    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await waitFor(() => expect(screen.queryByText('Loading the board…')).toBeNull());
    expect(screen.getByDisplayValue('Second page draft')).toBeTruthy();
    expect(screen.getByDisplayValue('Uncertain page-two reply')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Reply as owner' }));
    await waitFor(() => expect(active.board.reply).toHaveBeenCalledTimes(2));
    expect(vi.mocked(active.board.reply).mock.calls.map(([input]) => input.operationId)).toEqual([
      operationId,
      operationId,
    ]);
    expect(vi.mocked(active.board.show).mock.calls.at(-1)?.[0].threadId).toBe(secondId);
  });

  it('single-flights continuations and ignores their results after refresh', async () => {
    const active = runtime();
    const { body: _initialBody, ...initialThread } = thread;
    const categoriesPage = deferred<Awaited<ReturnType<LocalRuntime['board']['categories']>>>();
    const threadsPage = deferred<Awaited<ReturnType<LocalRuntime['board']['list']>>>();
    const repliesPage = deferred<Awaited<ReturnType<LocalRuntime['board']['show']>>>();
    vi.mocked(active.board.categories).mockImplementation(async (input) =>
      input?.cursor
        ? categoriesPage.promise
        : { categories: [category], nextCursor: 'categories-2', boardRevision: 2 }
    );
    vi.mocked(active.board.list).mockImplementation(async (input) =>
      input.cursor
        ? threadsPage.promise
        : {
            threads: [{ ...initialThread, replyCount: 1, activitySequence: 2 }],
            nextCursor: 'threads-2',
            boardRevision: 2,
          }
    );
    vi.mocked(active.board.show).mockImplementation(async (input) =>
      input.replyCursor
        ? repliesPage.promise
        : { thread, replies: [reply], nextCursor: 'replies-2', boardRevision: 2 }
    );
    mount(active);
    await screen.findByRole('heading', { name: 'Current work' });
    await userEvent.click(screen.getByRole('button', { name: 'More categories' }));
    await userEvent.click(screen.getByRole('button', { name: 'More categories' }));
    await userEvent.click(screen.getByRole('button', { name: 'More threads' }));
    await userEvent.click(screen.getByRole('button', { name: 'More threads' }));
    await userEvent.click(screen.getByRole('button', { name: 'More replies' }));
    await userEvent.click(screen.getByRole('button', { name: 'More replies' }));
    expect(
      vi.mocked(active.board.categories).mock.calls.filter(([input]) => input?.cursor)
    ).toHaveLength(1);
    expect(vi.mocked(active.board.list).mock.calls.filter(([input]) => input.cursor)).toHaveLength(
      1
    );
    expect(
      vi.mocked(active.board.show).mock.calls.filter(([input]) => input.replyCursor)
    ).toHaveLength(1);

    await userEvent.click(screen.getByRole('button', { name: 'Refresh board' }));
    await waitFor(() => expect(screen.queryByText('Loading the board…')).toBeNull());
    const staleCategory = { kind: 'repository', repositoryId: 'github.com/Old/Category' } as const;
    categoriesPage.resolve({ categories: [staleCategory], nextCursor: null, boardRevision: 2 });
    const { body: _body, ...staleThread } = thread;
    threadsPage.resolve({
      threads: [
        {
          ...staleThread,
          id: replyId,
          threadId: replyId,
          title: 'Stale thread',
          replyCount: 0,
          activitySequence: 3,
        },
      ],
      nextCursor: null,
      boardRevision: 2,
    });
    repliesPage.resolve({
      thread,
      replies: [{ ...reply, id: '55555555-5555-4555-8555-555555555555', body: 'Stale reply' }],
      nextCursor: null,
      boardRevision: 2,
    });
    await Promise.resolve();
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: staleCategory.repositoryId })).toBeNull();
      expect(screen.queryByText('Stale thread')).toBeNull();
      expect(screen.queryByText('Stale reply')).toBeNull();
    });
  });
});
