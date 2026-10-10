import { expect, it } from 'vite-plus/test';
import { ProposalActions, type ProposalOutcome } from '../src/proposal-actions.js';
import type { ThreadBinding } from '../src/thread-store.js';
import type { AskBinding } from '../src/ask-panel.js';
import type { ThreadView } from '../src/thread-records.js';
import { destination as makeDestination } from './ask-fixtures.js';

const destination = makeDestination();
function scenario(fail?: string) {
  const events: string[] = [],
    outcomes: ProposalOutcome[] = [];
  const ref = { writer: crypto.randomUUID(), id: crypto.randomUUID() };
  let current = true;
  let final = false;
  const view = {
    ref,
    threadId: ref.id,
    revision: '1',
    deleted: false,
    comments: [],
    anchor: null,
    proposal: {
      proposalId: ref.id,
      title: 'Use the smaller heading',
      body: 'Details',
      proposer: { machineId: destination.machine, agentId: destination.agent, label: 'Old name' },
    },
  } as unknown as ThreadView;
  const step = async (name: string) => {
    events.push(name);
    if (fail === name) throw new Error(name);
  };
  const discussion = {
    async decideProposal() {
      await step('decision');
      final = true;
    },
    async reply(_ref: unknown, body: string) {
      await step('comment');
      expect(body).toBe('Approved: Use the smaller heading');
      return {
        thread: ref,
        message: { writer: ref.writer, id: crypto.randomUUID() },
        threadRevision: '1',
        messageRevision: '1',
      };
    },
  } as unknown as ThreadBinding;
  const ask = {
    async destinations() {
      await step('directory');
      return [{ ...destination, agentName: 'Renamed agent' }];
    },
    async prepare(input: Parameters<AskBinding['prepare']>[0]) {
      await step('prepare');
      expect(input.destination.agent).toBe(destination.agent);
      expect(input.context?.thread).toEqual(ref);
      expect(input.url).toBe('https://test.test/page');
      return {
        async send() {
          await step('send');
          return { state: 'accepted', adopted: true };
        },
      };
    },
  } as unknown as AskBinding;
  const actions = new ProposalActions({
    thread: () => view,
    discussion: () => discussion,
    ask: () => ask,
    asks: () => [],
    current: () => current,
    title: () => 'Page',
    url: () => 'https://test.test/page',
    changed: (_id, outcome) => outcomes.push(outcome),
  });
  return {
    actions,
    events,
    outcomes,
    final: () => final,
    stop: () => {
      current = false;
    },
  };
}
it('commits the final decision before comment and one UUID-addressed Ask, fencing double activation', async () => {
  const s = scenario();
  await Promise.all([s.actions.decide('id', 'approved'), s.actions.decide('id', 'declined')]);
  expect(s.events).toEqual(['decision', 'comment', 'directory', 'prepare', 'send']);
  expect(s.final()).toBe(true);
  expect(s.outcomes.at(-1)?.state).toBe('settled');
  await s.actions.decide('id', 'approved');
  expect(s.events.filter((e) => e === 'send')).toHaveLength(1);
});
it('does no work before a trusted caller invokes the action, and stopped admission does no work', async () => {
  const s = scenario();
  expect(s.events).toEqual([]);
  s.stop();
  await s.actions.decide('id', 'approved');
  expect(s.events).toEqual([]);
});
it('unknown decision publication outcome never prepares or resumes a notification', async () => {
  const s = scenario('decision');
  await s.actions.decide('id', 'approved');
  expect(s.events).toEqual(['decision']);
  expect(s.outcomes.at(-1)).toMatchObject({ state: 'uncertain', committed: false });
  await s.actions.decide('id', 'approved');
  expect(s.events).toEqual(['decision']);
});
for (const stage of ['comment', 'directory', 'prepare', 'send'])
  it(`retains the decision after ${stage} fails without automatic resend`, async () => {
    const s = scenario(stage);
    await s.actions.decide('id', 'approved');
    expect(s.final()).toBe(true);
    expect(s.outcomes.at(-1)?.committed).toBe(true);
    expect(s.events).toEqual(
      ['decision', 'comment', 'directory', 'prepare', 'send'].slice(
        0,
        ['decision', 'comment', 'directory', 'prepare', 'send'].indexOf(stage) + 1,
      ),
    );
    await s.actions.decide('id', 'approved');
    expect(s.events.filter((e) => e === 'decision')).toHaveLength(1);
    expect(s.events.filter((e) => e === 'send')).toHaveLength(stage === 'send' ? 1 : 0);
    if (stage === 'send') expect(s.outcomes.at(-1)?.failure?.uncertain).toBe(true);
  });
