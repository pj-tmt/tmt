import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { AGENT_PRESETS, checkPrAgents, parseAgentLines } from '../../scripts/pr-agent-check.mjs';
import { writeExecutable } from '../support/executable-fixture.mjs';

const head = 'a'.repeat(40);
const base = 'b'.repeat(40);
const repository = 'pj-tmt/tmt';
const valid = 'Agent: infra-1 codex-sol-high';
const pull = (number: number, body: string | null = valid, extra = {}) => ({
  number,
  body,
  title: 'ci: test',
  user: { login: 'benhsieh' },
  ...extra,
});
function reader(pulls: Record<number, unknown>, numbers = [1, 2]) {
  const calls: string[] = [];
  return {
    calls,
    git(args: string[]) {
      if (args[0] === 'merge-base') {
        expect(args).toEqual(['merge-base', '--all', 'refs/remotes/origin/main', head]);
        return base;
      }
      expect(args).toEqual(['log', '--reverse', '--format=%H%x09%s', `${base}..${head}`]);
      return numbers.map((n) => `${head}\tci: test (#${n})`).join('\n');
    },
    rest(endpoint: string) {
      calls.push(endpoint);
      const n = Number(endpoint.split('/').at(-1));
      return JSON.stringify(pulls[n]);
    },
  };
}
const prEvent = { pull_request: { number: 1, body: 'stale event body' } };
const queueEvent = { merge_group: { head_sha: head, base_sha: 'c'.repeat(40) } };

describe('visible Agent grammar', () => {
  it('owns exactly the starting presets', () => {
    expect(AGENT_PRESETS).toEqual([
      'codex-sol-high',
      'codex-sol-med',
      'codex-luna-med',
      'codex-luna-low',
      'claude-opus-med',
      'claude-sonnet-high',
      'claude-sonnet-xhigh',
    ]);
    expect(Object.isFrozen(AGENT_PRESETS)).toBe(true);
  });
  it('keeps the owning guide preset table equal to the parser policy', () => {
    const guide = readFileSync(
      new URL('../../../.agents/skills/tmt-release/references/native-release.md', import.meta.url),
      'utf8'
    );
    const section = guide.split('## PR Agent attribution\n')[1]?.split('\n## ')[0];
    expect(section).toBeDefined();
    const presets = [...(section ?? '').matchAll(/^\| ([a-z][a-z0-9-]*)\s+\|/gm)].map(
      (row) => row[1]
    );
    expect(presets).toEqual(AGENT_PRESETS);
  });
  it.each([
    ...AGENT_PRESETS.map((p) => `Agent: infra-1 ${p}`),
    'Agent: ben',
    'Agent: seat-2 vendor Model.2 high_effort',
  ])('accepts %s', (line) => {
    expect(parseAgentLines(line)).toEqual({ agents: [line.slice(7)], findings: [] });
  });
  it.each([
    '',
    'Agent:',
    'Agent: <seat> <preset>',
    'Agent: Infra-1 codex-sol-high',
    'Agent: -seat codex-sol-high',
    'Agent: seat unknown',
    'Agent: seat CODEX-SOL-HIGH',
    'agent: seat codex-sol-high',
    'Agent: seat vendor model',
    'Agent: seat vendor model high extra',
    'Agent: seat vendor/model x high',
    '- Agent: ben',
    '> Agent: ben',
  ])('refuses %s', (line) => {
    expect(parseAgentLines(line).findings.length).toBeGreaterThan(0);
  });
  it('counts multiple visible contributors and refuses a typo beside a valid line', () => {
    expect(parseAgentLines(`${valid}\nAgent: ux-1 claude-sonnet-xhigh`).agents).toHaveLength(2);
    expect(parseAgentLines(`${valid}\nAgent: ux-1 claude-sonnet-xhig`).findings).toEqual([
      'Invalid Agent line 2.',
    ]);
  });
  it('ignores fenced and commented lines including malformed examples', () => {
    const body = [
      '```text',
      'Agent: bad unknown',
      '```',
      '<!--',
      'Agent: missing',
      '-->',
      '~~~',
      'Agent: ben',
      '~~~',
      '<!-- Agent: nonsense -->',
      valid,
    ].join('\n');
    expect(parseAgentLines(body)).toEqual({ agents: ['infra-1 codex-sol-high'], findings: [] });
    expect(parseAgentLines('````\nAgent: ben\n```\nAgent: ben').agents).toEqual([]);
    expect(parseAgentLines('<!-- Agent: ben -->').agents).toEqual([]);
  });
  it('rejects unavailable bodies without inventing attribution', () => {
    for (const body of [null, undefined, {}, 2])
      expect(parseAgentLines(body).findings).toEqual(['Missing PR body.']);
  });
});

describe('current PR and cumulative queue evidence', () => {
  it('uses REST body instead of stale event and reads only the requested PR', () => {
    const source = reader({ 1: pull(1) });
    expect(
      checkPrAgents({ eventName: 'pull_request', event: prEvent, reader: source, repository })
    ).toEqual({ checked: 1, exempted: [], findings: [] });
    expect(source.calls).toEqual(['repos/pj-tmt/tmt/pulls/1']);
    expect(
      checkPrAgents({
        eventName: 'pull_request',
        event: { pull_request: { number: 1, body: valid } },
        reader: reader({ 1: pull(1, '') }),
        repository,
      }).findings
    ).toHaveLength(1);
  });
  it.each([
    ['tmt-ci-bot[bot]', 'chore(main): release 1.0.0', true],
    ['benhsieh', 'chore(main): release 1.0.0', false],
    ['other[bot]', 'chore(main): release 1.0.0', false],
    ['tmt-ci-bot[bot]', 'ci: release', false],
  ])('requires bot and release title together (%s, %s)', (login, title, exempt) => {
    const result = checkPrAgents({
      eventName: 'pull_request',
      event: prEvent,
      repository,
      reader: reader({ 1: pull(1, null, { user: { login }, title }) }),
    });
    expect(result.exempted).toEqual(exempt ? [1] : []);
    expect(result.findings).toHaveLength(exempt ? 0 : 1);
  });
  it('finds one missing line among queued PRs and deduplicates REST reads', () => {
    const source = reader({ 1: pull(1), 2: pull(2, '') }, [1, 2, 1]);
    const result = checkPrAgents({
      eventName: 'merge_group',
      event: queueEvent,
      reader: source,
      repository,
    });
    expect(result).toEqual({
      checked: 2,
      exempted: [],
      findings: [{ number: 2, findings: ['No valid visible Agent line.'] }],
    });
    expect(source.calls).toEqual(['repos/pj-tmt/tmt/pulls/1', 'repos/pj-tmt/tmt/pulls/2']);
    expect(
      checkPrAgents({
        eventName: 'merge_group',
        event: queueEvent,
        reader: reader({ 1: pull(1), 2: pull(2) }),
        repository,
      }).findings
    ).toEqual([]);
  });
  it('fails closed on malformed, missing or unavailable current evidence', () => {
    for (const evidence of [
      null,
      {},
      pull(2),
      pull(1, valid, { user: null }),
      pull(1, valid, { title: 5 }),
      pull(1, valid, { body: 2 }),
    ]) {
      expect(() =>
        checkPrAgents({
          eventName: 'pull_request',
          event: prEvent,
          repository,
          reader: reader({ 1: evidence }),
        })
      ).toThrow();
    }
    expect(() =>
      checkPrAgents({
        eventName: 'pull_request',
        event: prEvent,
        repository,
        reader: {
          git: () => '',
          rest: () => {
            throw new Error('unavailable');
          },
        },
      })
    ).toThrow('unavailable');
    expect(() =>
      checkPrAgents({ eventName: 'pull_request', event: {}, repository, reader: reader({}) })
    ).toThrow('PR number');
  });
  it('allows report-only only for merge groups and retains their findings', () => {
    expect(() =>
      checkPrAgents({
        eventName: 'pull_request',
        event: prEvent,
        repository,
        reader: reader({ 1: pull(1) }),
        reportOnly: true,
      })
    ).toThrow('only for merge groups');
    expect(
      checkPrAgents({
        eventName: 'merge_group',
        event: queueEvent,
        repository,
        reader: reader({ 1: pull(1, ''), 2: pull(2) }),
        reportOnly: true,
      }).findings
    ).toHaveLength(1);
  });
});

it('real entrypoint keeps PR errors red and queue observation green with visible reports', () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'tmt-pr-agents-'));
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  try {
    writeExecutable(
      path.join(dir, 'gh'),
      `#!${process.execPath}\nconsole.log(require('node:fs').readFileSync(process.env.AGENT_PULL, 'utf8'));\n`,
      0o700
    );
    writeExecutable(
      path.join(dir, 'git'),
      `#!${process.execPath}\nconsole.log(process.argv[2] === 'merge-base' ? '${base}' : '${head}\\tci: test (#1)');\n`,
      0o700
    );
    const eventPath = path.join(dir, 'event');
    const bodyPath = path.join(dir, 'pull');
    const summary = path.join(dir, 'summary');
    function invoke(eventName: string, body: unknown, reportOnly = false) {
      writeFileSync(eventPath, JSON.stringify(eventName === 'merge_group' ? queueEvent : prEvent));
      writeFileSync(bodyPath, JSON.stringify(body));
      writeFileSync(summary, '');
      const result = spawnSync(
        process.execPath,
        [
          path.join(root, 'typescript/scripts/pr-agent-check.mjs'),
          ...(reportOnly ? ['--report-only'] : []),
        ],
        {
          env: {
            ...process.env,
            PATH: `${dir}${path.delimiter}${process.env.PATH}`,
            GITHUB_EVENT_NAME: eventName,
            GITHUB_EVENT_PATH: eventPath,
            GITHUB_REPOSITORY: repository,
            GITHUB_STEP_SUMMARY: summary,
            AGENT_PULL: bodyPath,
          },
          encoding: 'utf8',
          timeout: 30_000,
        }
      );
      expect(result.error).toBeUndefined();
      expect(readFileSync(summary, 'utf8')).toBe(result.stdout);
      return result;
    }
    expect(invoke('pull_request', pull(1)).status).toBe(0);
    const missing = invoke('pull_request', pull(1, ''));
    expect(missing.status).toBe(1);
    expect(missing.stdout).toContain('No valid visible Agent line.');
    const observing = invoke('merge_group', pull(1, ''), true);
    expect(observing.status).toBe(0);
    expect(observing.stdout).toContain('Findings do not fail this merge group.');
    const unavailable = invoke('merge_group', {}, true);
    expect(unavailable.status).toBe(0);
    expect(unavailable.stdout).toContain('Agent evidence unavailable');
    expect(invoke('pull_request', {}, true).status).toBe(1);
    expect(invoke('pull_request', {}).status).toBe(1);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
