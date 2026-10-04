import { spawnSync, type SpawnSyncReturns } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { componentMap } from '../../scripts/ci-scope.mjs';
import {
  checkPullRequestTitle,
  checkQueueTitles,
  conventionalPrTitle,
  pendingQueueSubjects,
  type QueueReader,
} from '../../scripts/pr-title-check.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const head = 'a'.repeat(40);
const base = 'b'.repeat(40);
const earlier = 'c'.repeat(40);
const event = { merge_group: { head_sha: head, base_sha: earlier } };
// Exact non-conventional titles read via REST from #993/#1034 on 2026-10-02.
const lostTitles = [
  'Freeze Office releases and official extension acquisition',
  'Squad board: completed-request token rate meter',
];
const row = (sha: string, title: string, number: number) => `${sha}\t${title} (#${number})`;
function reader(log: string): QueueReader {
  return {
    git(args) {
      if (args[0] === 'merge-base') {
        expect(args).toEqual(['merge-base', '--all', 'refs/remotes/origin/main', head]);
        return base;
      }
      expect(args).toEqual(['log', '--reverse', '--format=%H%x09%s', `${base}..${head}`]);
      return log;
    },
  };
}

describe('conventional squash title syntax', () => {
  it.each([
    'fix: repair lock',
    'feat(cli): add option',
    'docs(site): change text',
    'chore(main): release 5.0.0-alpha.40',
    'feat!: change contract',
    'refactor(core)!: change contract',
    'test(process): cover lifecycle (#123)',
  ])('accepts %s', (title) => expect(conventionalPrTitle(title)).toBe(true));
  it.each([
    ...lostTitles,
    '',
    'fix repair lock',
    'fix:',
    'fix: ',
    'fix:  leading space',
    'fix(): empty scope',
    'fix(core: unclosed scope',
    'fix(core))!: extra parenthesis',
    'fix(core)!!: duplicate marker',
    'fix: first\nsecond',
  ])('reports %s', (title) => expect(conventionalPrTitle(title)).toBe(false));
  it('rejects missing title data', () => expect(conventionalPrTitle(undefined)).toBe(false));
});

describe('shared cumulative queue subject evidence', () => {
  it('reports earlier invalid subjects despite a conventional tip and the later event base', () => {
    const source = reader(
      [row(earlier, lostTitles[0], 993), row(head, 'docs(site): update text', 1035)].join('\n')
    );
    expect(checkQueueTitles({ event, reader: source })).toEqual({
      checked: 2,
      findings: [{ sha: earlier, title: lostTitles[0], number: 993 }],
    });
  });
  it('removes only the final GitHub suffix, preserving title references', () => {
    expect(
      pendingQueueSubjects({ event, reader: reader(row(head, 'fix: retain (#99)', 100)) })
    ).toEqual([{ sha: head, title: 'fix: retain (#99)', number: 100 }]);
    expect(
      checkQueueTitles({ event, reader: reader(row(head, 'fix: retain (#99)', 100)) })
    ).toEqual({ checked: 1, findings: [] });
  });
  it('rejects missing, malformed and oversized evidence at the shared reader boundary', () => {
    for (const log of [
      '',
      'invalid',
      row('invalid', 'fix: change', 1),
      `${head}\tno PR suffix`,
      Array.from({ length: 41 }, (_, i) => row(head, 'fix: change', i + 1)).join('\n'),
    ]) {
      expect(() => pendingQueueSubjects({ event, reader: reader(log) })).toThrow();
    }
    expect(() => pendingQueueSubjects({ event: {}, reader: reader('') })).toThrow('event data');
    for (const evidence of ['', 'not a sha', `${base}\n${earlier}`]) {
      expect(() => pendingQueueSubjects({ event, reader: { git: () => evidence } })).toThrow(
        'cumulative queue base'
      );
    }
  });
});

function reportFixture(
  run: (context: {
    directory: string;
    invoke: (options?: {
      log?: string;
      gitStatus?: number;
      summaryDirectory?: boolean;
      missingSummary?: boolean;
      missingEvent?: boolean;
      eventName?: string;
      missingToken?: boolean;
    }) => { result: SpawnSyncReturns<string>; summary: string };
  }) => void
) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-pr-title-report-'));
  try {
    writeExecutable(
      path.join(directory, 'git'),
      `#!${process.execPath}\nconst fs = require('node:fs');\nif (process.env.GIT_FIXTURE_STATUS !== '0') process.exit(Number(process.env.GIT_FIXTURE_STATUS));\nconsole.log(process.argv[2] === 'merge-base' ? '${base}' : fs.readFileSync(process.env.GIT_FIXTURE_LOG, 'utf8'));\n`,
      0o700
    );
    // Any accidental REST/GraphQL title read makes this fixture fail visibly.
    writeExecutable(
      path.join(directory, 'gh'),
      '#!/bin/sh\necho unexpected-REST >&2\nexit 29\n',
      0o700
    );
    const eventPath = path.join(directory, 'event.json');
    writeFileSync(eventPath, JSON.stringify(event));
    const invoke = (
      options: {
        log?: string;
        gitStatus?: number;
        summaryDirectory?: boolean;
        missingSummary?: boolean;
        missingEvent?: boolean;
        eventName?: string;
        missingToken?: boolean;
      } = {}
    ) => {
      const summaryPath = options.summaryDirectory ? directory : path.join(directory, 'summary');
      if (!options.summaryDirectory) writeFileSync(summaryPath, '');
      const logPath = path.join(directory, 'log');
      writeFileSync(logPath, options.log ?? row(head, 'fix: change', 1));
      const result = spawnSync(
        process.execPath,
        [path.join(root, 'typescript/scripts/pr-title-check.mjs')],
        {
          env: {
            ...process.env,
            PATH: `${directory}:${process.env.PATH}`,
            GITHUB_REPOSITORY: 'pj-tmt/tmt',
            GITHUB_TOKEN: options.missingToken ? '' : 'fixture',
            GITHUB_EVENT_NAME: options.eventName ?? 'merge_group',
            GITHUB_EVENT_PATH: options.missingEvent ? path.join(directory, 'missing') : eventPath,
            GITHUB_STEP_SUMMARY: options.missingSummary ? '' : summaryPath,
            GIT_FIXTURE_LOG: logPath,
            GIT_FIXTURE_STATUS: String(options.gitStatus ?? 0),
          },
          encoding: 'utf8',
          timeout: 5000,
        }
      );
      return {
        result,
        summary:
          options.summaryDirectory || options.missingSummary
            ? ''
            : readFileSync(summaryPath, 'utf8'),
      };
    };
    run({ directory, invoke });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('real report-only command exit status and job summary', () => {
  it('writes both historical findings and exits zero so the step cannot fail the group', () =>
    reportFixture(({ invoke }) => {
      const { result, summary } = invoke({
        log: [row(earlier, lostTitles[0], 993), row(head, lostTitles[1], 1034)].join('\n'),
      });
      expect(result.status).toBe(0);
      expect(result.signal).toBeNull();
      expect(result.stderr).toBe('');
      expect(result.stdout).toBe(summary);
      expect(summary).toContain('2 finding(s)');
      for (const title of lostTitles) expect(summary).toContain(title);
      expect(summary).toContain(earlier);
      expect(summary).toContain('Findings do not fail this merge group');
    }));
  it('writes a clean report for conventional titles and exits zero', () =>
    reportFixture(({ invoke }) => {
      const { result, summary } = invoke();
      expect(result.status).toBe(0);
      expect(summary).toContain('Checked 1 queued title(s); 0 finding(s)');
    }));
  it('reports unavailable evidence while remaining zero for the entire observation phase', () =>
    reportFixture(({ invoke }) => {
      for (const options of [
        { log: '' },
        { log: 'invalid' },
        { gitStatus: 2 },
        { missingEvent: true },
      ]) {
        const { result, summary } = invoke(options);
        expect(result.status).toBe(0);
        expect(result.signal).toBeNull();
        expect(summary).toContain('Title evidence unavailable');
        expect(summary).toContain('does not fail the merge group');
      }
    }));
  it('refuses an unsupported event instead of reporting a clean result', () =>
    reportFixture(({ invoke }) => {
      const { result } = invoke({ eventName: 'push' });
      expect(result.status).toBe(1);
      expect(result.stdout).toContain('runs only on pull requests and merge groups');
    }));
  it('reads local queue evidence without any token or REST title reads', () =>
    reportFixture(({ invoke }) => {
      const { result, summary } = invoke({ missingToken: true });
      expect(result.status).toBe(0);
      expect(summary).toContain('Checked 1 queued title(s); 0 finding(s)');
    }));
  it('keeps summary I/O failures non-blocking and retains the report in stdout', () =>
    reportFixture(({ invoke }) => {
      for (const options of [{ summaryDirectory: true }, { missingSummary: true }]) {
        const { result } = invoke(options);
        expect(result.status).toBe(0);
        expect(result.stdout).toContain('Conventional PR titles (report-only)');
        expect(result.stderr).toContain('Title report summary unavailable');
      }
    }));
  it('escapes untrusted title markup in the job summary', () =>
    reportFixture(({ invoke }) => {
      const { result, summary } = invoke({ log: row(head, '<img src=x> & title', 1) });
      expect(result.status).toBe(0);
      expect(summary).toContain('&lt;img src=x&gt; &amp; title');
      expect(summary).not.toContain('<img');
    }));
});

// Real released component roots from the checked-in map, so a map change is noticed here.
const remotePath = 'extensions/tmt-remote/rust/src/lib.rs';
describe('pull-request title gate', () => {
  const map = componentMap();
  const check = (title: string, paths: string[]) => checkPullRequestTitle({ title, paths, map });
  it('rejects a non-conventional title on a released component and names it', () => {
    const result = check('Keep Remote door origins stable and add status', [remotePath]);
    expect(result.ok).toBe(false);
    expect(result.components).toContain('tmt-remote');
  });
  it('accepts a conventional title on the same change and any title off released paths', () => {
    expect(check('fix(tmt-remote): keep door origins stable', [remotePath]).ok).toBe(true);
    expect(
      check('Reword the Office readme', ['extensions/tmt-office/README.md']).components
    ).toEqual([]);
    expect(check('Reword the Office readme', ['extensions/tmt-office/README.md']).ok).toBe(true);
    expect(check('', ['extensions/tmt-office/README.md']).ok).toBe(true);
  });
  it('rejects an empty or capitalized title when a released component changes', () => {
    for (const title of ['', 'Squad: capitalized type'])
      expect(check(title, ['rust/crates/tmt-cli/src/main.rs']).ok).toBe(false);
  });
});

function gateFixture(
  run: (
    invoke: (options: {
      title: string;
      files: string[];
      restStatus?: number;
    }) => SpawnSyncReturns<string> & { summary: string }
  ) => void
) {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-pr-title-gate-'));
  try {
    writeExecutable(
      path.join(directory, 'git'),
      `#!${process.execPath}\nconst fs = require('node:fs');\nif (process.argv[2] !== 'diff' || process.argv[5] !== '-z' || process.argv[6] !== '${base}...${head}') process.exit(2);\nprocess.stdout.write(fs.readFileSync(process.env.GIT_FIXTURE_FILES, 'utf8'));\n`,
      0o700
    );
    // The event payload carries a stale title; only REST holds the current one.
    writeExecutable(
      path.join(directory, 'gh'),
      `#!${process.execPath}\nif (process.env.GH_FIXTURE_STATUS !== '0') process.exit(Number(process.env.GH_FIXTURE_STATUS));\nif (process.argv.slice(2).join(' ') !== 'api repos/pj-tmt/tmt/pulls/77') process.exit(3);\nprocess.stdout.write(JSON.stringify({ title: process.env.GH_FIXTURE_TITLE }));\n`,
      0o700
    );
    const eventPath = path.join(directory, 'event.json');
    writeFileSync(
      eventPath,
      JSON.stringify({
        pull_request: {
          number: 77,
          title: 'fix: stale original title',
          base: { sha: base },
          head: { sha: head },
        },
      })
    );
    run(({ title, files, restStatus = 0 }) => {
      const filesPath = path.join(directory, 'files');
      const summaryPath = path.join(directory, 'summary');
      writeFileSync(filesPath, files.map((file) => `${file}\0`).join(''));
      writeFileSync(summaryPath, '');
      const result = spawnSync(
        process.execPath,
        [path.join(root, 'typescript/scripts/pr-title-check.mjs')],
        {
          env: {
            ...process.env,
            PATH: `${directory}:${process.env.PATH}`,
            GITHUB_REPOSITORY: 'pj-tmt/tmt',
            GITHUB_EVENT_NAME: 'pull_request',
            GITHUB_EVENT_PATH: eventPath,
            GITHUB_STEP_SUMMARY: summaryPath,
            GIT_FIXTURE_FILES: filesPath,
            GH_FIXTURE_TITLE: title,
            GH_FIXTURE_STATUS: String(restStatus),
          },
          encoding: 'utf8',
          timeout: 5000,
        }
      );
      return Object.assign(result, { summary: readFileSync(summaryPath, 'utf8') });
    });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe('real pull-request gate command', () => {
  it('fails on the current REST title, not the stale event title, and says how to recover', () =>
    gateFixture((invoke) => {
      const result = invoke({ title: 'Keep Remote door origins stable', files: [remotePath] });
      expect(result.status).toBe(1);
      expect(result.summary).toContain('tmt-remote');
      expect(result.summary).toContain('Keep Remote door origins stable');
      expect(result.summary).toContain('Edit the PR title');
      expect(result.summary).toContain('rerun Code quality');
      expect(result.stdout).toBe(result.summary);
    }));
  it('passes once the title is fixed and for a PR that changes no released component', () =>
    gateFixture((invoke) => {
      expect(
        invoke({ title: 'fix(tmt-remote): keep door origins stable', files: [remotePath] }).status
      ).toBe(0);
      const docsOnly = invoke({
        title: 'Reword the Office readme',
        files: ['extensions/tmt-office/README.md'],
      });
      expect(docsOnly.status).toBe(0);
      expect(docsOnly.summary).toContain('No released component changed');
    }));
  it('fails closed when the title cannot be read', () =>
    gateFixture((invoke) => {
      const result = invoke({ title: 'fix: x', files: [remotePath], restStatus: 4 });
      expect(result.status).toBe(1);
      expect(result.summary).toContain('Title evidence unavailable');
      expect(result.summary).toContain('Rerun Code quality');
    }));
});

describe('workflow wiring', () => {
  it('runs one checker on pull requests and merge groups without edited or an extra workflow', () => {
    const ci = readFileSync(path.join(root, '.github/workflows/ci.yml'), 'utf8');
    const step = ci
      .split('      - name: Check conventional PR titles\n')[1]
      .split('      - name: Require selected Office verification')[0];
    expect(step).not.toContain('if:');
    expect(step).toContain('GITHUB_TOKEN: ${{ github.token }}');
    expect(step).toContain('run: node typescript/scripts/pr-title-check.mjs');
    expect(ci.split('  pull_request:\n')[1].split('  merge_group:')[0]).not.toContain('edited');
  });
});
