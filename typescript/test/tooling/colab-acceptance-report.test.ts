import { execFileSync } from 'node:child_process';
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

function runReport(change?: (result: FixtureResult) => void, missing = false) {
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
                  tests: [{ status: 'unexpected', expectedStatus: 'passed', results: [result] }],
                },
              ],
            },
          ],
        },
      ],
    };
    const input = path.join(root, 'private.json');
    if (!missing) writeFileSync(input, JSON.stringify(report));
    const stdout = execFileSync(process.execPath, [script], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, PLAYWRIGHT_JSON_OUTPUT_FILE: input, RUNNER_TEMP: root },
    });
    const text = readFileSync(path.join(root, 'colab-acceptance-results/outcomes.json'), 'utf8');
    expect(text + stdout).not.toContain(secret);
    expect(text + stdout).not.toContain(root);
    return { report: JSON.parse(text), stdout };
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

  it('writes empty evidence after setup failed before a report existed', () => {
    const { report, stdout } = runReport(undefined, true);
    expect(report).toEqual({ tests: [] });
    expect(stdout).toBe('');
  });
});
