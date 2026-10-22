import { spawnSync } from 'node:child_process';
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const script = fileURLToPath(new URL('../../scripts/colab-acceptance-report.mjs', import.meta.url));
const secret = 'PRIVATE_CREDENTIAL_SENTINEL';

interface FixtureResult {
  status: string;
  duration: number;
  errorLocation: { file: string; line: number | string; column: number };
  error: { message: string; stack: string };
  attachments: { name?: string; contentType?: string; body: string; path?: string }[];
  stdout: string[];
  stderr: string[];
}

function runReport(
  change?: (result: FixtureResult) => void,
  missing = false,
  execution: unknown = { state: 'completed', exitCode: 1 },
  raw?: string
) {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt-colab-report-')));
  try {
    const source = path.join(root, 'extensions/tmt-colab/typescript/app/acceptance');
    mkdirSync(source, { recursive: true });
    writeFileSync(path.join(source, 'page-size.spec.ts'), '// Public fixture\n');
    const result: FixtureResult = {
      status: 'failed',
      duration: 6078,
      errorLocation: { file: path.join(source, 'page-size.spec.ts'), line: 38, column: 7 },
      error: { message: `Timeout 5000ms exceeded\n${secret}`, stack: secret },
      attachments: [{ body: secret }],
      stdout: [secret],
      stderr: [secret],
    };
    change?.(result);
    const report = {
      suites: [
        {
          suites: [
            {
              specs: [
                {
                  file: path.basename(result.errorLocation.file),
                  title: 'Exact source fixture',
                  tests: [
                    {
                      status: result.status === 'passed' ? 'expected' : 'unexpected',
                      expectedStatus: 'passed',
                      results: [result],
                    },
                  ],
                },
              ],
            },
          ],
        },
      ],
    };
    const input = path.join(root, 'private.json');
    if (!missing) writeFileSync(input, raw ?? JSON.stringify(report));
    mkdirSync(path.join(root, 'colab-acceptance-private'));
    writeFileSync(
      path.join(root, 'colab-acceptance-private/execution.json'),
      JSON.stringify(execution)
    );
    const child = spawnSync(process.execPath, [script], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, PLAYWRIGHT_JSON_OUTPUT_FILE: input, RUNNER_TEMP: root },
    });
    expect(child.error).toBeUndefined();
    const stdout = child.stdout;
    const text = readFileSync(path.join(root, 'colab-acceptance-results/outcomes.json'), 'utf8');
    expect(text + stdout).not.toContain(secret);
    expect(text + stdout).not.toContain(root);
    const parsed = JSON.parse(text);
    expect(child.status).toBe(parsed.run.status === 'passed' ? 0 : 1);
    expect(child.stderr).not.toContain(secret);
    return { report: parsed, stdout };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe('public Colab acceptance evidence', () => {
  const diagnostic = {
    root: 'messages',
    kind: 'ask-state',
    streamId: '30000000-0000-4000-8000-000000000001',
    submittedSeq: '7',
    admittedSeq: '4',
    leaderReceipt: true,
  };
  const attachment = (value: unknown) => ({
    name: 'colab-own-publication',
    contentType: 'application/json',
    body: Buffer.from(JSON.stringify(value)).toString('base64'),
  });

  it('retains only a typed own-publication attachment, never generic private errors', () => {
    const { report } = runReport((result) => {
      result.attachments.push(attachment(diagnostic));
    });
    expect(report.tests[0].results[0].ownPublications).toEqual([diagnostic]);
  });

  it.each(['attachment-publication', 'edit-request'])(
    'retains %s only under its intents root',
    (kind) => {
      const row = { ...diagnostic, root: 'intents', kind };
      const { report } = runReport((result) => {
        result.attachments.push(attachment(row), attachment({ ...row, root: 'messages' }));
      });
      expect(report.tests[0].results[0].ownPublications).toEqual([row]);
    }
  );

  it.each([
    { key: secret },
    { value: secret },
    { url: secret },
    { root: secret },
    { kind: secret },
    { streamId: secret },
    { submittedSeq: secret },
    { admittedSeq: 4 },
    { submittedSeq: '18446744073709551616' },
    { leaderReceipt: secret },
  ])('drops diagnostics outside the closed schema: %j', (change) => {
    const { report } = runReport((result) => {
      result.attachments.push(attachment({ ...diagnostic, ...change }));
    });
    expect(report.tests[0].results[0]).not.toHaveProperty('ownPublications');
  });

  it('does not follow attachment paths or copy unnamed JSON attachments', () => {
    const { report } = runReport((result) => {
      result.attachments.push(
        { ...attachment(diagnostic), name: 'private-diagnostic', path: secret },
        { ...attachment(diagnostic), body: '', path: secret }
      );
    });
    expect(report.tests[0].results[0]).not.toHaveProperty('ownPublications');
  });

  it.each([{ contentType: 'text/plain' }, { body: 'not base64' }, { body: 'A'.repeat(1028) }])(
    'drops malformed or oversized diagnostic attachments: %j',
    (change) => {
      const { report } = runReport((result) => {
        result.attachments.push({ ...attachment(diagnostic), ...change });
      });
      expect(report.tests[0].results[0]).not.toHaveProperty('ownPublications');
    }
  );

  describe('Live-status diagnostic', () => {
    const live = { state: 'opening', frame: true, visibility: 'hidden', ended: 'session' };
    const liveAttachment = (value: unknown) => ({ ...attachment(value), name: 'colab-live-state' });

    it('retains only a closed state, frame presence, visibility and ended cause', () => {
      const { report } = runReport((result) => {
        result.attachments.push(liveAttachment(live));
      });
      expect(report.tests[0].results[0].liveStates).toEqual([live]);
    });

    it.each([
      { state: secret },
      { state: 'Opening page…' },
      { frame: 'yes' },
      { visibility: secret },
      { ended: secret },
      { ended: 'Session ended' },
      { text: secret },
    ])('drops a diagnostic outside the closed schema: %j', (change) => {
      const { report } = runReport((result) => {
        result.attachments.push(liveAttachment({ ...live, ...change }));
      });
      expect(report.tests[0].results[0]).not.toHaveProperty('liveStates');
    });

    it.each([
      'access',
      'revoked',
      'expired',
      'mismatch',
      'unpaired',
      'gone',
      'session',
      'large',
      'capacity',
      'generic',
      'none',
      'other',
    ])('retains the closed ended cause %s', (ended) => {
      const { report } = runReport((result) => {
        result.attachments.push(liveAttachment({ ...live, ended }));
      });
      expect(report.tests[0].results[0].liveStates).toEqual([{ ...live, ended }]);
    });

    it('drops a row with a missing field and a diagnostic on a passing result', () => {
      const { frame: _frame, ...partial } = live;
      const { ended: _ended, ...withoutEnded } = live;
      for (const row of [partial, withoutEnded])
        expect(
          runReport((result) => {
            result.attachments.push(liveAttachment(row));
          }).report.tests[0].results[0]
        ).not.toHaveProperty('liveStates');
      expect(
        runReport((result) => {
          result.status = 'passed';
          result.attachments.push(liveAttachment(live));
        }).report.tests[0].results[0]
      ).not.toHaveProperty('liveStates');
    });

    it('does not accept the own-publication name for a live state or the reverse', () => {
      const { report } = runReport((result) => {
        result.attachments.push(attachment(live), liveAttachment(diagnostic));
      });
      expect(report.tests[0].results[0]).not.toHaveProperty('liveStates');
      expect(report.tests[0].results[0]).not.toHaveProperty('ownPublications');
    });
  });

  it('does not turn a passing result into publication failure evidence', () => {
    const { report } = runReport((result) => {
      result.status = 'passed';
      result.attachments.push(attachment(diagnostic));
    });
    expect(report.tests[0].results[0]).not.toHaveProperty('ownPublications');
  });

  it('retains a located failure and numeric poll timeout without private payloads', () => {
    const { report, stdout } = runReport();
    const expected = {
      status: 'failed',
      duration: 6078,
      failure: {
        file: 'acceptance/page-size.spec.ts',
        line: 38,
        column: 7,
        timeoutMs: 5000,
      },
    };
    expect(report.tests[0].results).toEqual([expected]);
    expect(JSON.parse(stdout)).toEqual(report.tests[0]);
  });

  it.each(['external', 'missing', 'symlink', 'line', 'column'])('rejects %s locations', (kind) => {
    const { report } = runReport((result) => {
      if (kind === 'external') result.errorLocation.file = `/private/${secret}`;
      if (kind === 'missing')
        result.errorLocation.file = result.errorLocation.file.replace('page-size', 'absent');
      if (kind === 'symlink') {
        writeFileSync(result.errorLocation.file + '.private', secret);
        rmSync(result.errorLocation.file);
        symlinkSync(result.errorLocation.file + '.private', result.errorLocation.file);
      }
      if (kind === 'line') result.errorLocation.line = '38';
      if (kind === 'column') result.errorLocation.column = 0;
    });
    expect(report.tests[0].results).toEqual([{ status: 'failed', duration: 6078 }]);
  });

  it('bounds a timeout while retaining the source location', () => {
    const { report } = runReport((result) => {
      result.error.message = 'Timeout 1800001ms exceeded';
    });
    expect(report.tests[0].results[0].failure).toEqual({
      file: 'acceptance/page-size.spec.ts',
      line: 38,
      column: 7,
    });
  });

  it.each(['deadline', 'interrupted', 'running', 'startup-failed', 'runner-failed'])(
    'keeps a %s verdict incomplete even when the flushed case passed',
    (state) => {
      const { report } = runReport(
        (result) => {
          result.status = 'passed';
        },
        false,
        { state, private: secret }
      );
      expect(report.run).toEqual({ status: 'incomplete', reason: state });
      expect(report.tests).toHaveLength(1);
    }
  );

  it('sanitizes malformed JSON and invalid execution without copying private parse errors', () => {
    expect(
      runReport(undefined, false, { state: 'completed', exitCode: 0 }, secret).report.run
    ).toEqual({ status: 'incomplete', reason: 'malformed-report' });
    expect(runReport(undefined, false, { state: secret }).report.run).toEqual({
      status: 'incomplete',
      reason: 'invalid-execution',
    });
  });

  it('requires complete execution and report evidence for a passing verdict', () => {
    expect(
      runReport(
        (result) => {
          result.status = 'passed';
        },
        false,
        { state: 'completed', exitCode: 0 }
      ).report.run
    ).toEqual({ status: 'passed', reason: 'complete' });
  });

  it('retains an ordinary failure as failed and an interrupted result as incomplete', () => {
    expect(runReport().report.run).toEqual({ status: 'failed', reason: 'complete' });
    expect(
      runReport((result) => {
        result.status = 'interrupted';
      }).report.run
    ).toEqual({ status: 'incomplete', reason: 'interrupted-report' });
  });

  it('marks missing final JSON incomplete instead of publishing an empty successful run', () => {
    const { report, stdout } = runReport(undefined, true);
    expect(report).toEqual({ run: { status: 'incomplete', reason: 'missing-report' }, tests: [] });
    expect(JSON.parse(stdout)).toEqual({ run: report.run });
  });
});
