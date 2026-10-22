import { spawnSync } from 'node:child_process';
import {
  existsSync,
  openSync,
  closeSync,
  ftruncateSync,
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
import {
  loadSoakDictionary,
  policy,
} from '../../../extensions/tmt-colab/typescript/app/acceptance/harness/soak-literals.mjs';
import {
  projectSoakFailure,
  readSoakTail,
} from '../../../extensions/tmt-colab/typescript/app/acceptance/harness/soak-diagnostics.js';

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

/** Reject lexical escapes before any filesystem operation, then check existing real ancestry. */
function fixturePath(
  root: string,
  file: string,
  resolve: (file: string) => string = realpathSync
): string {
  const candidate = path.resolve(file);
  const inside = (value: string) => {
    const relative = path.relative(root, value);
    return relative !== '..' && !relative.startsWith('..' + path.sep) && !path.isAbsolute(relative);
  };
  if (!inside(candidate)) throw new Error('Fixture path escapes its temporary root');
  let ancestor = candidate;
  for (;;) {
    try {
      if (!inside(resolve(ancestor)))
        throw new Error('Fixture realpath escapes its temporary root');
      return candidate;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      ancestor = path.dirname(ancestor);
    }
  }
}

function runReport(
  change?: (result: FixtureResult, root: string) => void,
  missing = false,
  execution: unknown = { state: 'completed', exitCode: 1 },
  raw?: string,
  title = 'Exact source fixture'
) {
  const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt-colab-report-')));
  try {
    const source = path.join(root, 'extensions/tmt-colab/typescript/app/acceptance');
    mkdirSync(fixturePath(root, source), { recursive: true });
    writeFileSync(fixturePath(root, path.join(source, 'page-size.spec.ts')), '// Public fixture\n');
    writeFileSync(
      fixturePath(root, path.join(source, 'connection-soak.spec.ts')),
      '// Public fixture\n'
    );
    const result: FixtureResult = {
      status: 'failed',
      duration: 6078,
      errorLocation: { file: path.join(source, 'page-size.spec.ts'), line: 38, column: 7 },
      error: { message: `Timeout 5000ms exceeded\n${secret}`, stack: secret },
      attachments: [{ body: secret }],
      stdout: [secret],
      stderr: [secret],
    };
    change?.(result, root);
    const report = {
      suites: [
        {
          suites: [
            {
              specs: [
                {
                  file: path.basename(result.errorLocation.file),
                  title,
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
    if (!missing) writeFileSync(fixturePath(root, input), raw ?? JSON.stringify(report));
    mkdirSync(fixturePath(root, path.join(root, 'colab-acceptance-private')));
    writeFileSync(
      fixturePath(root, path.join(root, 'colab-acceptance-private/execution.json')),
      JSON.stringify(execution)
    );
    const child = spawnSync(process.execPath, [script], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, PLAYWRIGHT_JSON_OUTPUT_FILE: input, RUNNER_TEMP: root },
    });
    expect(child.error).toBeUndefined();
    const stdout = child.stdout;
    const outcomeFile = path.join(root, 'colab-acceptance-results/outcomes.json');
    expect(existsSync(outcomeFile), child.stderr.slice(0, 4096)).toBe(true);
    const text = readFileSync(outcomeFile, 'utf8');
    expect(text + stdout).not.toContain(secret);
    expect(text + stdout).not.toContain(root);
    const parsed = JSON.parse(text);
    expect(child.status).toBe(parsed.run.status === 'passed' ? 0 : 1);
    expect(child.stderr).not.toContain(secret);
    const diagnosticPath = path.join(root, 'colab-soak-diagnostics/soak-failure.json');
    const diagnostic = existsSync(diagnosticPath) ? readFileSync(diagnosticPath, 'utf8') : null;
    return { report: parsed, stdout, diagnostic };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe('public Colab acceptance evidence', () => {
  it('rejects an outside fixture path before any filesystem call', () => {
    let calls = 0;
    const resolve = () => {
      calls++;
      throw new Error('Filesystem must not be reached');
    };
    expect(() =>
      fixturePath('/private/tmp/fixture-root', '/private/PRIVATE_CREDENTIAL_SENTINEL', resolve)
    ).toThrow('Fixture path escapes');
    expect(calls).toBe(0);
  });

  it('rejects fixture writes through a symlink outside the temporary root', () => {
    const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt-colab-guard-')));
    const outside = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt-colab-outside-')));
    try {
      symlinkSync(outside, fixturePath(root, path.join(root, 'link')));
      expect(() => fixturePath(root, path.join(root, 'link', 'private'))).toThrow(
        'Fixture realpath escapes'
      );
      expect(existsSync(path.join(outside, 'private'))).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(outside, { recursive: true, force: true });
    }
  });

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
    const { report } = runReport((result, root) => {
      if (kind === 'external') result.errorLocation.file = `/private/${secret}`;
      if (kind === 'missing')
        result.errorLocation.file = result.errorLocation.file.replace('page-size', 'absent');
      if (kind === 'symlink') {
        writeFileSync(fixturePath(root, result.errorLocation.file + '.private'), secret);
        rmSync(fixturePath(root, result.errorLocation.file));
        symlinkSync(
          fixturePath(root, result.errorLocation.file + '.private'),
          fixturePath(root, result.errorLocation.file)
        );
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

describe('failed synthetic soak evidence', () => {
  it('retains a separate bounded diagnostic while keeping public outcomes unchanged', () => {
    const { diagnostic, report } = runReport(
      (result) => {
        result.errorLocation.file = path.join(
          path.dirname(result.errorLocation.file),
          'connection-soak.spec.ts'
        );
        result.attachments.push({
          name: 'colab-soak-failure',
          contentType: 'application/json',
          body: Buffer.from(
            JSON.stringify({
              version: 1,
              scenario: 'detached-eviction-refused-tab',
              error: { code: 'CONFLICT', message: 'CONFLICT' },
              spans: [{ name: 'locator.click', startMs: 12, durationMs: 3 }],
              logs: [],
              children: [],
              alive: { socket: false, door: false },
              assertion: 'peer-live-after-refusal',
              observedMs: 16,
            })
          ).toString('base64'),
        });
      },
      false,
      { state: 'completed', exitCode: 1 },
      undefined,
      'connection soak: detached eviction and a refused new tab leave the attached page usable'
    );
    expect(diagnostic).not.toBeNull();
    const retained = JSON.parse(diagnostic!);
    expect(retained.error).toEqual({ code: 'CONFLICT', message: 'CONFLICT', unrecognized: false });
    expect(retained.spans).toEqual([{ name: 'locator.click', startMs: 12, durationMs: 3 }]);
    expect(retained.assertion.id).toBe('peer-live-after-refusal');
    expect(report.tests[0].results[0]).not.toHaveProperty('spans');
    expect(report.tests[0].results[0]).not.toHaveProperty('error');
    expect(Buffer.byteLength(diagnostic!)).toBeLessThanOrEqual(16_384);
  });
});

describe('soak artifact selection', () => {
  it.each(['passed', 'failed'])('never publishes evidence from an unrelated %s case', (status) => {
    const result = runReport(
      (result) => {
        result.status = status;
        result.attachments = [
          {
            name: policy.attachment,
            contentType: 'application/json',
            body: Buffer.from(
              JSON.stringify({ version: 1, scenario: policy.id, observedMs: 1 })
            ).toString('base64'),
          },
        ];
      },
      false,
      { state: 'completed', exitCode: status === 'passed' ? 0 : 1 }
    );
    expect(result.diagnostic).toBeNull();
  });
  it('does not publish the named soak attachment from a passing case', () => {
    const result = runReport(
      (result) => {
        result.status = 'passed';
        result.errorLocation.file = path.join(
          path.dirname(result.errorLocation.file),
          'connection-soak.spec.ts'
        );
        result.attachments = [
          {
            name: policy.attachment,
            contentType: 'application/json',
            body: Buffer.from(
              JSON.stringify({ version: 1, scenario: policy.id, observedMs: 1 })
            ).toString('base64'),
          },
        ];
      },
      false,
      { state: 'completed', exitCode: 0 },
      undefined,
      policy.scenario
    );
    expect(result.diagnostic).toBeNull();
  });
  it('does not follow the soak attachment path', () => {
    const result = runReport(
      (result) => {
        result.errorLocation.file = path.join(
          path.dirname(result.errorLocation.file),
          'connection-soak.spec.ts'
        );
        result.attachments = [
          {
            name: policy.attachment,
            contentType: 'application/json',
            body: '',
            path: '/private/' + secret,
          },
        ];
      },
      false,
      { state: 'completed', exitCode: 1 },
      undefined,
      policy.scenario
    );
    expect(result.diagnostic).toBeNull();
  });
});

describe('closed soak diagnostic projection', () => {
  const dictionary = loadSoakDictionary();
  const sample = () => ({
    version: 1,
    scenario: policy.id,
    observedMs: 40_000,
    assertion: 'peer-live-after-refusal',
    error: { code: 'CONFLICT', message: 'CONFLICT' },
    spans: [{ name: 'locator.click', startMs: 39_000, durationMs: 20 }],
    logs: [{ source: 'remote', event: 'error', code: 'INVALID', message: 'INVALID', atMs: 39_010 }],
    children: [{ role: 'remote', alive: false, exitCode: 7, signal: null }],
    alive: { socket: false, door: false },
  });
  it('retains fixed assertion text, code timing/span and owned child exit information', () => {
    const value = projectSoakFailure(sample(), dictionary)!;
    expect(value.assertion).toEqual({
      id: 'peer-live-after-refusal',
      text: dictionary.assertions['peer-live-after-refusal'],
    });
    expect(value.logs[0]).toEqual({
      source: 'remote',
      event: 'error',
      code: 'INVALID',
      message: 'INVALID',
      unrecognized: false,
      atMs: 39_010,
      span: 'locator.click',
    });
    expect(value.children[0]).toEqual({
      index: 0,
      role: 'remote',
      alive: false,
      exitCode: 7,
      signal: null,
    });
    expect(value.alive).toEqual({ socket: false, door: false, method: 'owned-resource-probe' });
  });
  it.each([
    'Bearer NEGATIVE_BEARER',
    'session=NEGATIVE_COOKIE',
    'Authorization: NEGATIVE_AUTH',
    'https://invalid.test/?cap=NEGATIVE_CAP#NEGATIVE_FRAGMENT',
    'NEGATIVE_NETWORK_PAYLOAD',
  ])('never retains arbitrary private strings: %s', (secret) => {
    const value = sample();
    const malicious = {
      ...value,
      [secret]: secret,
      attachmentPath: secret,
      error: { ...value.error, Authorization: secret, cookie: secret, url: secret, body: secret },
      spans: [
        { ...value.spans[0], params: { value: secret }, subtitle: secret, unknown: secret },
        { name: secret, startMs: 1, durationMs: 1 },
      ],
      logs: [{ ...value.logs[0], headers: secret, request: secret }],
      assertion: { id: value.assertion, text: secret },
      children: [{ ...value.children[0], signal: secret, path: secret }],
    };
    const text = JSON.stringify(projectSoakFailure(malicious, dictionary));
    expect(text).not.toContain(secret);
    const unknown = projectSoakFailure(
      { ...malicious, error: { code: secret, message: secret } },
      dictionary
    )!;
    expect(unknown.error).toEqual({ code: null, message: null, unrecognized: true });
  });
  it('keeps only the bounded last-30-second API timeline and marks discarded spans/tails', () => {
    const value = projectSoakFailure(
      {
        ...sample(),
        spans: Array.from({ length: 129 }, (_, i) => ({
          name: 'page.goto',
          startMs: 39_000 + i,
          durationMs: 1,
        })),
        logs: Array.from({ length: 41 }, (_, i) => ({ ...sample().logs[0], atMs: 39_000 + i })),
      },
      dictionary
    )!;
    expect(value.spans).toHaveLength(128);
    expect(value.spans[0].startMs).toBe(39_001);
    expect(value.logs).toHaveLength(40);
    expect(value.logs[0].atMs).toBe(39_001);
    expect(value.truncated).toMatchObject({ spans: true, logs: true });
    const cutoff = projectSoakFailure(
      {
        ...sample(),
        spans: [
          { name: 'page.goto', startMs: 0, durationMs: 50 },
          { name: 'page.goto', startMs: 5_000, durationMs: 20_000 },
          { name: 'page.goto', startMs: -1, durationMs: 1 },
          { name: 'page.goto', startMs: NaN, durationMs: Infinity },
        ],
      },
      dictionary
    )!;
    expect(cutoff.spans).toEqual([{ name: 'page.goto', startMs: 10_000, durationMs: 15_000 }]);
    expect(cutoff.truncated.spans).toBe(true);
  });
  it('caps serialized bytes, preserves truncation across both projections and drops unknown cases', () => {
    const value = {
      ...sample(),
      children: Array.from({ length: 40 }, () => sample().children[0]),
      spans: Array.from({ length: 128 }, () => ({
        name: 'chromium.launchPersistentContext',
        startMs: 39_000,
        durationMs: 1,
      })),
      logs: Array.from({ length: 40 }, () => ({
        ...sample().logs[0],
        message: 'Decoder failed or exceeded its time budget',
      })),
    };
    const first = projectSoakFailure(value, dictionary)!;
    expect(Buffer.byteLength(JSON.stringify(first))).toBeLessThanOrEqual(policy.maxBytes);
    expect(first.truncated.input).toBe(true);
    expect(first.truncated.bytes).toBe(true);
    expect(projectSoakFailure(first, dictionary)!.truncated).toEqual(first.truncated);
    expect(projectSoakFailure({ ...value, scenario: 'unrelated' }, dictionary)).toBeUndefined();
  });
  it('reads a byte-capped tail from the end and refuses outward symlinks and nonregular files', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'soak-tail-'));
    const outside = mkdtempSync(path.join(os.tmpdir(), 'soak-outside-'));
    try {
      writeFileSync(path.join(outside, 'private'), secret);
      symlinkSync(path.join(outside, 'private'), path.join(root, 'remote.stderr'));
      expect(readSoakTail(root, 'remote.stderr')).toMatchObject({ lines: [], available: false });
      expect(readSoakTail(root, '../private')).toMatchObject({ lines: [], available: false });
      mkdirSync(path.join(root, 'colab.stderr'));
      expect(readSoakTail(root, 'colab.stderr')).toMatchObject({ lines: [], available: false });
      rmSync(path.join(root, 'remote.stderr'));
      const file = path.join(root, 'remote.stderr');
      const fd = openSync(file, 'w');
      try {
        ftruncateSync(fd, 1_000_000);
      } finally {
        closeSync(fd);
      }
      writeFileSync(file, '\n' + Array.from({ length: 41 }, (_, i) => String(i)).join('\n'), {
        flag: 'a',
      });
      const tail = readSoakTail(root, 'remote.stderr');
      expect(tail.available).toBe(true);
      expect(tail.truncated).toBe(true);
      expect(tail.lines).toEqual(Array.from({ length: 40 }, (_, i) => String(i + 1)));
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(outside, { recursive: true, force: true });
    }
  });
  it('imports only named static literals and fails owner drift rather than widening the dictionary', () => {
    const repo = new URL('../../../', import.meta.url);
    const read = (file: string) => readFileSync(new URL(file, repo), 'utf8');
    const fold = 'extensions/tmt-colab/typescript/app/src/fold.ts';
    const literal = 'Decoder failed or exceeded its time budget';
    for (const replacement of [
      "new Error('changed')",
      "new Error(String('" + literal + "'))",
      'new Error(`Decoder ${"changed"}`)',
    ]) {
      expect(() =>
        loadSoakDictionary((file) =>
          file === fold
            ? read(file).replace("new Error('" + literal + "')", replacement)
            : read(file)
        )
      ).toThrow('Missing static literal');
    }
    expect(() =>
      loadSoakDictionary((file) => {
        if (file === fold) throw new Error('Missing named owner');
        return read(file);
      })
    ).toThrow('Missing named owner');
    const rust = 'extensions/tmt-remote/rust/tmt-remote/src/error.rs';
    expect(() =>
      loadSoakDictionary((file) =>
        file === rust
          ? read(file).replace('Self::new("REMOTE_IO",', 'Changed::new("REMOTE_IO",')
          : read(file)
      )
    ).toThrow('Missing static literal');
    const widened = loadSoakDictionary((file) =>
      file === fold ? read(file) + '\nnew Error("UNLISTED_OWNER_LITERAL");' : read(file)
    );
    expect(widened.messages.has('UNLISTED_OWNER_LITERAL')).toBe(false);
    expect([...widened.messages].every((value) => typeof value === 'string')).toBe(true);
    const scenario = 'extensions/tmt-colab/typescript/app/acceptance/connection-soak.spec.ts';
    expect(() =>
      loadSoakDictionary((file) =>
        file === scenario
          ? read(file).replace(
              "'The SDK directory exactly matches the native identity set.'",
              "String('The SDK directory exactly matches the native identity set.')"
            )
          : read(file)
      )
    ).toThrow('Non-static assertion: directory');
  });
});

describe('filtered workspace compiler installs', () => {
  it.each(['tooling', 'app'])(
    'loads the same owner vocabulary with only the %s compiler installed',
    (owner) => {
      const root = realpathSync(mkdtempSync(path.join(realpathSync('/tmp'), 'tmt-soak-install-')));
      const repository = fileURLToPath(new URL('../../../', import.meta.url));
      const harness = 'extensions/tmt-colab/typescript/app/acceptance/harness';
      const compiler = path.resolve(
        path.dirname(fileURLToPath(import.meta.resolve('typescript'))),
        '..'
      );
      try {
        const files = [
          ...policy.owners.map((entry) => entry.file),
          'extensions/tmt-colab/typescript/app/acceptance/connection-soak.spec.ts',
          `${harness}/soak-policy.json`,
          `${harness}/soak-literals.mjs`,
        ];
        for (const file of new Set(files)) {
          const target = fixturePath(root, path.join(root, file));
          mkdirSync(fixturePath(root, path.dirname(target)), { recursive: true });
          writeFileSync(target, readFileSync(path.join(repository, file)));
        }
        const home = owner === 'tooling' ? 'typescript' : 'extensions/tmt-colab/typescript/app';
        const modules = fixturePath(root, path.join(root, home, 'node_modules'));
        mkdirSync(modules, { recursive: true });
        symlinkSync(compiler, fixturePath(root, path.join(modules, 'typescript')), 'dir');
        const loader = path.join(root, harness, 'soak-literals.mjs');
        const child = spawnSync(
          process.execPath,
          [
            '--input-type=module',
            '-e',
            `const {loadSoakDictionary}=await import(${JSON.stringify(loader)}); const value=loadSoakDictionary(); console.log(JSON.stringify({codes:[...value.codes].sort(),messages:[...value.messages].sort(),assertions:value.assertions}));`,
          ],
          { cwd: root, encoding: 'utf8', timeout: 10000, env: { ...process.env, NODE_PATH: '' } }
        );
        expect(child.error).toBeUndefined();
        expect(child.status, child.stderr).toBe(0);
        const dictionary = loadSoakDictionary();
        expect(JSON.parse(child.stdout)).toEqual({
          codes: [...dictionary.codes].sort(),
          messages: [...dictionary.messages].sort(),
          assertions: dictionary.assertions,
        });
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    }
  );
});

describe('soak Core-wrapper observation', () => {
  it.each(['0', '1'])('forwards exact stdout and exit status with diagnostics=%s', (enabled) => {
    const root = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'tmt-acc-wrapper-')));
    try {
      const barrier = fixturePath(root, path.join(root, 'barrier'));
      mkdirSync(barrier);
      const child = fixturePath(root, path.join(root, 'child.cjs'));
      const output = JSON.stringify({ error: { code: 'PANE_NOT_FOUND', message: secret } });
      writeFileSync(
        child,
        `process.stdout.write(${JSON.stringify(output)}); process.exitCode = 7;`
      );
      const wrapper = fileURLToPath(
        new URL(
          '../../../extensions/tmt-colab/typescript/app/acceptance/harness/core-barrier.mjs',
          import.meta.url
        )
      );
      const result = spawnSync(process.execPath, [wrapper, child], {
        input: '',
        encoding: 'utf8',
        timeout: 10000,
        env: {
          ...process.env,
          TMT_ACCEPTANCE_REAL_TMT: process.execPath,
          TMT_ACCEPTANCE_BARRIER_DIR: barrier,
          TMT_ACCEPTANCE_SOAK_DIAGNOSTICS: enabled,
        },
      });
      expect(result.error).toBeUndefined();
      expect(result.status).toBe(7);
      expect(result.stdout).toBe(output);
      const tail = readSoakTail(root, 'soak-native.stderr');
      if (enabled === '0') expect(tail.available).toBe(false);
      else {
        const records = tail.lines.map((line) => JSON.parse(line));
        expect(records[0]).toMatchObject({
          source: 'core',
          event: 'exit',
          exitCode: 7,
          signal: null,
        });
        expect(records[1]).toMatchObject({
          source: 'core',
          event: 'error',
          code: 'PANE_NOT_FOUND',
        });
        expect(records.every((row) => Number.isSafeInteger(row.atMs))).toBe(true);
        expect(tail.lines.join('')).not.toContain(child);
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
