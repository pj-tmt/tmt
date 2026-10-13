import { describe, expect, it } from 'vite-plus/test';
import type { PageAsk } from '../src/ask-panel.js';
import type { CommentView, ThreadView } from '../src/thread-records.js';
import {
  ReplyNotificationObserver,
  projectReplyNotifications,
  type ReplySources,
} from '../src/reply-notifications.js';

const me = 'viewer';
const classification = { questionThreads: new Map<string, string | null>() };
function thread(writer = me, id = 'thread', chat = false): ThreadView {
  return {
    version: 1,
    kind: 'thread',
    spaceId: 'space',
    pageId: 'page',
    epoch: '1',
    senderDevice: writer,
    revision: '1',
    deleted: false,
    deviceName: writer,
    createdAt: '1',
    at: '1',
    resolved: false,
    threadId: chat ? writer : id,
    ref: { writer, id: chat ? writer : id },
    anchor: chat ? null : { exact: 'Canonical quote', prefix: '', suffix: '' },
    comments: [],
  };
}
function comment(t: ThreadView, writer = me, id = 'comment'): CommentView {
  return {
    version: 1,
    kind: 'comment',
    spaceId: t.spaceId,
    pageId: t.pageId,
    epoch: t.epoch,
    senderDevice: writer,
    revision: '1',
    deleted: false,
    deviceName: 'claimed viewer',
    at: '1',
    messageId: id,
    thread: t.ref,
    body: 'First reply line\nSecond line',
    ref: { writer, id },
  };
}
function ask(t?: ThreadView, writer = me): PageAsk {
  return {
    writer,
    operationId: 'operation',
    requestId: 'request',
    message: 'Own turn',
    agent: 'agent',
    agentName: 'claimed viewer',
    deviceName: 'claimed viewer',
    issuedAt: 1,
    machine: 'machine',
    state: 'accepted',
    canTrack: writer === me,
    reply: 'Agent first line\nMore reply',
    ...(t ? { thread: t.threadId, messageIds: [t.comments[0].messageId] } : {}),
  };
}
const project = (sources: ReplySources, questions = classification.questionThreads) =>
  projectReplyNotifications(sources, me, { questionThreads: questions });
const values = (sources: ReplySources) => [...project(sources).candidates.values()];

describe('admitted Chat and Thread reply notification sources', () => {
  it('routes own-ledger finals through the existing conversation association and retains exact quote/record identity', () => {
    const t = thread();
    t.comments = [comment(t)];
    expect(values({ threads: [t], asks: [ask(t)] })).toMatchObject([
      {
        destination: 'Thread',
        quote: 'Canonical quote',
        firstLine: 'Agent first line',
        thread: t.ref,
        recordKey: 'reply:viewer:operation',
      },
    ]);
    const chat = thread(me, 'unused', true);
    chat.comments = [comment(chat)];
    expect(values({ threads: [chat], asks: [ask(chat)] })[0]).toMatchObject({
      destination: 'Chat',
    });
    expect(values({ threads: [chat], asks: [ask(chat)] })[0]).not.toHaveProperty('quote');
    expect(values({ threads: [], asks: [ask()] })[0].destination).toBe('Chat');
    expect(values({ threads: [t], asks: [{ ...ask(t), messageIds: ['unrelated'] }] })).toEqual([]);
    expect(
      values({ threads: [t, { ...t, ref: { writer: 'other', id: t.threadId } }], asks: [ask(t)] }),
    ).toEqual([]);
  });
  it('uses authenticated Yours refs for participation and rejects own outgoing, other conversations and unavailable replies', () => {
    const t = thread('other');
    t.comments = [comment(t, me), comment(t, 'other', 'incoming')];
    expect(values({ threads: [t] })).toMatchObject([
      { destination: 'Thread', firstLine: 'First reply line', recordKey: 'comment:other:incoming' },
    ]);
    t.comments[0] = { ...t.comments[0], ref: { writer: 'stranger', id: 'comment' } };
    expect(values({ threads: [t], asks: [ask(t, 'stranger')] })).toEqual([]);
    const own = thread();
    own.comments = [comment(own, 'other')];
    expect(values({ threads: [own] })).toHaveLength(1);
    expect(values({ threads: [], asks: [ask()], askUnavailable: true })).toEqual([]);
    expect(
      projectReplyNotifications({ threads: [own], asks: [ask()] }, undefined, classification)
        .candidates.size,
    ).toBe(0);
  });
  it('classifies admitted Answer/Proposal associations once and never uses body labels as authority', () => {
    const q = thread();
    q.comments = [comment(q), comment(q, 'other', 'follow-up')];
    const a = ask(q);
    const key = JSON.stringify([q.ref.writer, q.ref.id]);
    const questions = { questionThreads: new Map([[key, 'question-one']]) };
    expect([
      ...projectReplyNotifications({ threads: [q], asks: [a] }, me, questions).candidates.values(),
    ]).toMatchObject([
      { destination: 'Answer', questionId: 'question-one' },
      { destination: 'Answer', questionId: 'question-one' },
    ]);
    expect(
      projectReplyNotifications({ threads: [q], asks: [a] }, me, {
        questionThreads: new Map([[key, null]]),
      }).candidates.size,
    ).toBe(0);
    q.proposal = {
      proposalId: q.threadId,
      title: 'Proposal',
      body: '',
      proposer: { machineId: 'machine', agentId: 'agent', label: 'Agent' },
    };
    expect(values({ threads: [q], asks: [a] })).toMatchObject([
      { destination: 'Proposal' },
      { destination: 'Proposal' },
    ]);
    delete q.proposal;
    q.comments[1].body = 'Answer to “question-one”: fake body classification';
    expect(values({ threads: [q] })[0].destination).toBe('Thread');
    const foreign = thread('foreign');
    foreign.comments = [comment(foreign, 'foreign')];
    foreign.proposal = {
      proposalId: foreign.threadId,
      title: 'Proposal',
      body: '',
      proposer: { machineId: me, agentId: me, label: me },
    };
    expect(values({ threads: [foreign], asks: [ask(foreign, 'foreign')] })).toEqual([]);
  });
  it('baselines Answer/Proposal history and never duplicates a follow-up after classification resolves', () => {
    for (const proposal of [false, true]) {
      const t = thread();
      t.comments = [comment(t)];
      if (proposal)
        t.proposal = {
          proposalId: t.threadId,
          title: 'P',
          body: '',
          proposer: { machineId: 'm', agentId: 'a', label: 'A' },
        };
      const sources = { threads: [t], asks: [] as PageAsk[] };
      const observer = new ReplyNotificationObserver(sources);
      const cls = { questionThreads: new Map([[JSON.stringify([t.ref.writer, t.ref.id]), 'q']]) };
      t.comments.push(comment(t, 'other', 'follow-up'));
      sources.asks = [ask(t)];
      if (!proposal)
        expect(
          observer.observe(
            sources,
            projectReplyNotifications(sources, me, { questionThreads: undefined }),
          ),
        ).toEqual([]);
      expect(observer.observe(sources, projectReplyNotifications(sources, me, cls))).toHaveLength(
        2,
      );
      expect(observer.observe(sources, projectReplyNotifications(sources, me, cls))).toEqual([]);
      expect(
        new ReplyNotificationObserver(sources).observe(
          sources,
          projectReplyNotifications(sources, me, cls),
        ),
      ).toEqual([]);
    }
  });
  it('silently withdraws deleted origins and refuses to use names to revive participation', () => {
    const t = thread();
    t.comments = [comment(t)];
    const a = ask(t);
    expect(values({ threads: [{ ...t, deleted: true }], asks: [a] })).toEqual([]);
    expect(
      values({ threads: [{ ...t, comments: [{ ...t.comments[0], deleted: true }] }], asks: [a] }),
    ).toEqual([]);
    const foreign = thread('not-me');
    foreign.comments = [comment(foreign, 'also-not-me')];
    expect(values({ threads: [foreign] })).toEqual([]);
  });
});

describe('page-local baselines and replay suppression', () => {
  it('baselines the first current binding view after a cached snapshot, then observes later finals', () => {
    const observer = new ReplyNotificationObserver({ asks: [], threads: [] });
    const t = thread();
    t.comments = [comment(t), comment(t, 'other', 'old')];
    const sources = { asks: [ask(t)], threads: [t] };
    observer.baselineFirstView(sources);
    expect(observer.observe(sources, project(sources))).toEqual([]);
    const unavailable = new ReplyNotificationObserver({ asks: [], threads: [] });
    unavailable.baselineFirstView({ askUnavailable: true });
    expect(unavailable.observe(sources, project(sources))).toEqual([]);
    t.comments.push(comment(t, 'other', 'later'));
    expect(observer.observe(sources, project(sources))).toHaveLength(1);
    expect(observer.observe(sources, project(sources))).toEqual([]);
  });
  it('baselines existing finals/comments and admits a later final once across edits, deletion/revival and redelivery', () => {
    const t = thread();
    t.comments = [comment(t), comment(t, 'other', 'old')];
    const a: PageAsk = { ...ask(t), reply: undefined };
    const sources: ReplySources = { asks: [a], threads: [t] };
    const observer = new ReplyNotificationObserver(sources);
    expect(observer.observe(sources, project(sources))).toEqual([]);
    a.reply = 'New final';
    const arrival = observer.observe(sources, project(sources));
    expect(arrival).toHaveLength(1);
    a.reply = 'Edited/redelivered text';
    expect(observer.observe(sources, project(sources))).toEqual([]);
    t.comments.push(comment(t, 'other', 'new'));
    expect(observer.observe(sources, project(sources))).toHaveLength(1);
    t.comments[2].revision = '2';
    t.comments[2].body = 'Edited comment';
    expect(observer.observe(sources, project(sources))).toEqual([]);
    t.comments[2].deleted = true;
    observer.observe(sources, project(sources));
    t.comments[2].deleted = false;
    expect(observer.observe(sources, project(sources))).toEqual([]);
    expect(new ReplyNotificationObserver(sources).observe(sources, project(sources))).toEqual([]);
  });
  it('does not turn the first available ledger or discussion read into historical arrivals', () => {
    const observer = new ReplyNotificationObserver({ askUnavailable: true });
    const t = thread();
    t.comments = [comment(t), comment(t, 'other', 'old')];
    const sources = { asks: [ask(t)], threads: [t] };
    expect(observer.observe(sources, project(sources))).toEqual([]);
    t.comments.push(comment(t, 'other', 'new'));
    expect(observer.observe(sources, project(sources))).toHaveLength(1);
  });
  it('defers classification during question resolution without losing a new ordinary thread reply', () => {
    const t = thread();
    t.comments = [comment(t)];
    const sources: ReplySources = { asks: [], threads: [t] };
    const observer = new ReplyNotificationObserver(sources);
    sources.asks = [ask(t)];
    const pending = projectReplyNotifications(sources, me, { questionThreads: undefined });
    expect(pending.deferred.size).toBe(1);
    expect(observer.observe(sources, pending)).toEqual([]);
    expect(observer.observe(sources, project(sources))).toHaveLength(1);
    expect(observer.observe(sources, project(sources))).toEqual([]);
  });
  it('does not retroactively notify a foreign conversation when the viewer joins later', () => {
    const t = thread('other');
    const sources = { threads: [t], asks: [] };
    const observer = new ReplyNotificationObserver(sources);
    t.comments.push(comment(t, 'other', 'old-foreign'));
    expect(observer.observe(sources, project(sources))).toEqual([]);
    t.comments.push(comment(t, me, 'joined'));
    expect(observer.observe(sources, project(sources))).toEqual([]);
    t.comments.push(comment(t, 'other', 'after-joining'));
    expect(observer.observe(sources, project(sources))).toHaveLength(1);
  });
});
