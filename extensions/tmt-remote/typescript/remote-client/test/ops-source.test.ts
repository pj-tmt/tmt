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
      id: '00000000-0000-4000-8000-000000000001',
      presence: 'active',
      activity: { state: 'working' },
      name: 'tmt-lead',
      squad: 'tmt-core',
      role: 'lead',
      state: 'working',
      task: 'review',
      waitingOnYou: 0,
    },
    {
      id: '00000000-0000-4000-8000-000000000002',
      presence: 'offline',
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

it('gives every agent state its own mark, so no two states look alike', () => {
  const marks = Object.values(AGENT_STATES).map((state) => state.mark);
  expect(new Set(marks).size).toBe(marks.length);
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
  // Only a browser the door names no owner mount for is asked to pair; a mount that did not
  // answer this time is retried, never a reason to pair again (#4018).
  expect(boardFromAnswer({ state: 'no-session' }).unavailable).toMatch(/Pair it from Remote/);
  const unreachable = boardFromAnswer({ state: 'unreachable' }).unavailable!;
  expect(unreachable).toBe('Your machine did not answer. This page tries again shortly.');
  expect(unreachable).not.toMatch(/[Pp]air/);
  // Each reason, an unknown one included, reads as its own sentence, never as an empty list.
  expect(texts.size).toBe(6);
  expect(answer({ version: 1, state: 'ok', squads: 'x', agents: [] }).unavailable).toMatch(
    /could not read/,
  );
});

it('keeps identity and plain-state evidence, and refuses missing or non-UUID member IDs', () => {
  const payload = structuredClone(ops[0]!.expect);
  const valid = answer(payload);
  expect(valid.agents[0]!.id).toBe('00000000-0000-4000-8000-000000000001');
  for (const id of [undefined, 'tmt-lead', 42]) {
    const malformed = structuredClone(payload) as { agents: { id?: unknown }[] };
    malformed.agents[0]!.id = id;
    expect(answer(malformed).unavailable).toMatch(/could not read/);
  }
});
