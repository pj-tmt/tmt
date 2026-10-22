import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { AGENT_STATES, boardFromAnswer, type OpsPayload } from '../src/ops-source.js';
import { ownerAnswer } from '../src/owner-reads.js';

const vectors = JSON.parse(
  readFileSync(new URL('../../../../../contracts/owner-reads-v1.json', import.meta.url), 'utf8'),
) as { cases: { name: string; target: string; expect: Record<string, unknown> }[] };
const ops = vectors.cases.filter((vector) => vector.target === 'ops');
const answer = (document: Record<string, unknown>) =>
  boardFromAnswer(ownerAnswer<OpsPayload>({ ...document, observedAtMs: 1 }));

it('turns the contract board into rows: lead first, a mark for each state, the board text kept', () => {
  const board = answer(ops[0]!.expect);
  expect(board.unavailable).toBeUndefined();
  expect(board.squads).toEqual([{ name: 'tmt-core', lead: 'tmt-lead', idleAlertMinutes: 30 }]);
  expect(board.agents).toEqual([
    {
      name: 'tmt-lead',
      squad: 'tmt-core',
      role: 'lead',
      state: 'working',
      task: 'review',
      waitingOnYou: 0,
    },
    {
      name: 'core-3',
      squad: 'tmt-core',
      role: 'member',
      // Offline wins over what it waits for; the board's own words stay beside the mark.
      state: 'offline',
      stateText: 'blocked: waits on CI now',
      task: '#4325',
      next: 'contract',
      pending: 'approve the plan',
      model: 'sonnet',
      waitingOnYou: 2,
    },
  ]);
  expect(board.schedules).toEqual([
    {
      id: 'c1',
      squad: 'tmt-core',
      when: 'cron 35 * * * *',
      what: 'Hourly pass',
      owner: 'tmt-core-pm',
    },
  ]);
  for (const agent of board.agents) expect(AGENT_STATES[agent.state].word).toBeTruthy();
});

it('keeps a failed schedule read apart from no timers, and an empty board ok', () => {
  const failed = ops.find((vector) => vector.name.includes('timers null'))!;
  expect(answer(failed.expect)).toEqual({
    squads: [],
    agents: [],
    schedules: null,
    truncated: false,
  });
  const empty = ops.find((vector) => vector.name.includes('no squads'))!;
  expect(answer(empty.expect)).toEqual({ squads: [], agents: [], schedules: [], truncated: false });
});

it('says why there is no board, with what to do, for every reason and no answer at all', () => {
  const texts = new Set<string>();
  for (const reason of [
    'not_installed',
    'core_unavailable',
    'timeout',
    'read_failed',
    'too_large',
    'later',
  ]) {
    const board = answer({ version: 1, state: 'unavailable', reason });
    expect(board).toMatchObject({ agents: [], squads: [], schedules: [] });
    texts.add(board.unavailable!);
  }
  const unreachable = boardFromAnswer({ state: 'unreachable' });
  expect(unreachable.unavailable).toMatch(/Pair it from Remote/);
  // Each reason, an unknown one included, reads as its own sentence, never as an empty list.
  expect(texts.size).toBe(6);
  expect(answer({ version: 1, state: 'ok', squads: 'x', agents: [] }).unavailable).toMatch(
    /could not read/,
  );
});
