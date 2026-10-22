import { expect, it } from 'vite-plus/test';
import { mockDigestSource } from '../src/digest-mock.js';
import { digestSource, USAGE_RANGES } from '../src/digest-source.js';
import { AGENT_STATES, mockOpsSource, opsSource } from '../src/ops-source.js';

it('reads no sample data in the product: the default sources name the terminal command', async () => {
  for (const range of USAGE_RANGES) {
    const report = await digestSource.report(range);
    expect(report).toMatchObject({ range, sample: false });
    expect(report.document).toBeUndefined();
    expect(report.unavailable).toBe(
      'Usage is not in the browser yet. Open Digest in `tmt ops ui`.',
    );
  }
  const board = await opsSource.board();
  expect(board).toMatchObject({
    agents: [],
    squads: [],
    schedules: [],
    settings: [],
    sample: false,
  });
  expect(board.unavailable).toBe(
    'The Ops board is not in the browser yet. Run `tmt ops ui` in a terminal.',
  );
});

it('marks the mock sources (tests, gallery, dev) as sample data for every range and the board', async () => {
  expect(USAGE_RANGES).toEqual(['24h', '7d', '30d']);
  for (const range of USAGE_RANGES) {
    const report = await mockDigestSource.report(range);
    expect(report.sample).toBe(true);
    expect(report.range).toBe(range);
    expect(report.document?.since).toBe(range);
  }
  const board = await mockOpsSource.board();
  expect(board.sample).toBe(true);
  for (const agent of board.agents) {
    expect(board.squads.some((squad) => squad.name === agent.squad)).toBe(true);
    expect(AGENT_STATES[agent.state].word).toBeTruthy();
  }
});
