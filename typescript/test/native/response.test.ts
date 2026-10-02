import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import Database from 'better-sqlite3';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { EXPECTED_NATIVE_SCHEMA_VERSION, initializeHistoricalDatabase } from './storage-fixture.js';
import { installTmuxTripwire } from './tmux-tripwire.js';
import {
  expectError,
  expectJsonSuccess,
  parseWholeStdout,
  runCli,
  type Sandbox,
  withSandbox,
} from '../support/cli-process.js';
import {
  MAX_RESPONSE_BYTES,
  RESPONSE_SERVER,
  compactReceipt,
  responseSnapshot,
  removeAttempt,
  seedResponse,
  schemaVersion,
  v1Receipt,
  type SeededResponse,
  type ResponseEndpoint,
} from './response-fixture.js';

describe('native response authorization and retention boundaries', () => {
  it('identifies an unwritable data directory before reply, result, or talk effects', async () => {
    await withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const seeded = seedResponse(sandbox.database, 'permission-retry');
      const before = responseSnapshot(sandbox.database, seeded.requestId);
      const reply = [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.compactReceipt,
        '--message',
        'exact retry body',
        '--json',
      ];
      chmodSync(sandbox.globalDir, 0o500);
      try {
        for (const args of [
          reply,
          ['result', seeded.requestId, '--json'],
          ['talk', '%14', 'must not send', '--json'],
        ]) {
          const failed = await runCli(sandbox, args);
          expect(failed.status).toBe(1);
          expectError(failed, 'STORAGE_NOT_WRITABLE');
          const message = (parseWholeStdout(failed).error as { message: string }).message;
          expect(message).toContain(sandbox.globalDir);
          expect(message).toContain('sandbox');
          if (args === reply) expect(message).toContain('Nothing was stored');
          if (args[0] === 'talk') expect(message).toContain('No message was sent');
        }
      } finally {
        chmodSync(sandbox.globalDir, 0o700);
      }
      expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(before);
      const accepted = await runCli(sandbox, reply);
      expect(accepted.status).toBe(0);
      expect(responseSnapshot(sandbox.database, seeded.requestId).response).toMatchObject({
        body: 'exact retry body',
      });
    });
  });

  it('keeps non-permission storage errors on the existing response code', async () => {
    await withSandbox(async (sandbox) => {
      mkdirSync(sandbox.globalDir, { recursive: true });
      writeFileSync(sandbox.database, 'not a SQLite database');
      const failed = await runCli(sandbox, ['result', 'missing-request', '--json']);
      expect(failed.status).toBe(1);
      expectError(failed, 'RESPONSE_ERROR');
      expect(readFileSync(sandbox.database, 'utf8')).toBe('not a SQLite database');
    });
  });

  it.each(['v1', 'v2'] as const)(
    'rejects every %s request/attempt/endpoint mismatch without mutation',
    async (version) => {
      await withSandbox(async (sandbox) => {
        await initializeSchema(sandbox);
        const seeded = seedResponse(sandbox.database, 'all-fences');
        const before = responseSnapshot(sandbox.database, seeded.requestId);
        const encode = version === 'v1' ? v1Receipt : compactReceipt;
        const cases: Array<{
          field: string;
          request?: string;
          attempt?: string;
          endpoint?: Partial<ResponseEndpoint>;
          v1Error: string;
        }> = [
          { field: 'request', request: 'other-request', v1Error: 'RESPONSE_RECEIPT_MISMATCH' },
          { field: 'attempt', attempt: 'other-attempt', v1Error: 'RESPONSE_ATTEMPT_MISMATCH' },
          ...Object.entries({
            serverId: 'other-server',
            socketPath: '/tmp/other.sock',
            serverPid: 1235,
            serverStartTime: 'other-start',
            paneId: '%2',
            panePid: 5679,
          }).map(([field, value]) => ({
            field,
            endpoint: { [field]: value },
            v1Error: 'RESPONSE_RECIPIENT_MISMATCH',
          })),
        ];
        for (const candidate of cases) {
          const receipt = encode(
            candidate.request ?? seeded.requestId,
            candidate.attempt ?? seeded.attemptId,
            { ...RESPONSE_SERVER, ...candidate.endpoint }
          );
          const result = await runCli(sandbox, [
            'reply',
            seeded.requestId,
            '--receipt',
            receipt,
            '--message',
            'must not commit',
            '--json',
          ]);
          expect(result.status, candidate.field).toBe(1);
          expectError(result, version === 'v1' ? candidate.v1Error : 'RESPONSE_RECEIPT_MISMATCH');
          expect(responseSnapshot(sandbox.database, seeded.requestId), candidate.field).toEqual(
            before
          );
        }
        const accepted = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          encode(seeded.requestId, seeded.attemptId),
          '--message',
          'authorized',
          '--json',
        ]);
        expect(accepted.status).toBe(0);
        expect(responseSnapshot(sandbox.database, seeded.requestId).response).toMatchObject({
          body: 'authorized',
          body_bytes: 10,
        });
      });
    }
  );

  it('prunes an expired retained body while keeping the acceptance marker against resurrection', async () => {
    await withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const prepared = Date.now() - 2 * 86_400_000;
      const seeded = seedResponse(sandbox.database, 'expired-retained-body', { nowMs: prepared });
      const submitted = await runCli(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.compactReceipt,
        '--message',
        'retained but expired',
        '--json',
      ]);
      expect(submitted.status).toBe(0);
      const writer = new Database(sandbox.database);
      try {
        writer.transaction(() => {
          writer
            .prepare(
              'UPDATE request_attempts SET retention_days = 1, response_submitted_at_ms = ?, message_expires_at_ms = ? WHERE request_id = ?'
            )
            .run(prepared + 1, prepared + 86_400_000, seeded.requestId);
          writer
            .prepare(
              'UPDATE request_responses SET submitted_at_ms = ?, response_expires_at_ms = ? WHERE request_id = ?'
            )
            .run(prepared + 1, prepared + 86_400_001, seeded.requestId);
        })();
      } finally {
        writer.close();
      }
      const before = responseSnapshot(sandbox.database, seeded.requestId);
      const result = await runCli(sandbox, ['result', seeded.requestId, '--json']);
      expect(result.status).toBe(3);
      expectError(result, 'RESPONSE_NOT_AVAILABLE');
      expect(result.stdout).not.toContain('retained but expired');
      const after = responseSnapshot(sandbox.database, seeded.requestId);
      expect(after).toEqual({
        ...before,
        attempt: { ...before.attempt, message_text: null, message_bytes: null },
        response: undefined,
      });
      const retry = await runCli(
        sandbox,
        ['reply', seeded.requestId, '--receipt', seeded.compactReceipt, '--stdin', '--json'],
        { stdin: Buffer.from('retained but expired') }
      );
      expect(retry.status).toBe(1);
      expectError(retry, 'RESPONSE_EXPIRED');
      expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(after);
    });
  });
});

async function initializeSchema(sandbox: Sandbox): Promise<void> {
  expectJsonSuccess(await runCli(sandbox, ['identity', 'list', '--json']), { identities: [] });
  expect(existsSync(sandbox.database)).toBe(true);
}

function temporaryFile(sandbox: Sandbox, name: string, bytes: string | Uint8Array): string {
  const file = path.join(sandbox.root, name);
  writeFileSync(file, bytes);
  return file;
}

function malformedConfig(sandbox: Sandbox): void {
  mkdirSync(sandbox.globalDir, { recursive: true });
  writeFileSync(sandbox.globalConfig, '{ malformed global config');
  writeFileSync(sandbox.localConfig, '{ malformed local config');
}

function assertSubmitted(
  result: Awaited<ReturnType<typeof runCli>>,
  seeded: SeededResponse,
  expectedBodyBytes?: number
): {
  readonly submittedAtMs: number;
} {
  expect(result.status).toBe(0);
  expect(result.stderr).toBe('');
  const document = parseWholeStdout(result) as Record<string, unknown>;
  expect(document).toMatchObject({
    status: 'submitted',
    requestId: seeded.requestId,
    bodyBytes: expect.any(Number),
    submittedAtMs: expect.any(Number),
  });
  expect(Object.keys(document).sort()).toEqual([
    'bodyBytes',
    'requestId',
    'status',
    'submittedAtMs',
  ]);
  if (expectedBodyBytes !== undefined) expect(document.bodyBytes).toBe(expectedBodyBytes);
  return { submittedAtMs: document.submittedAtMs as number };
}

function assertCompleted(
  result: Awaited<ReturnType<typeof runCli>>,
  seeded: SeededResponse,
  body: string,
  submittedAtMs?: number
): void {
  expect(result.status).toBe(0);
  expect(result.stderr).toBe('');
  const document = parseWholeStdout(result) as Record<string, unknown>;
  expect(document).toMatchObject({
    status: 'completed',
    requestId: seeded.requestId,
    response: body,
    bodyBytes: Buffer.byteLength(body),
    submittedAtMs: submittedAtMs ?? expect.any(Number),
  });
  expect(Object.keys(document).sort()).toEqual([
    'bodyBytes',
    'requestId',
    'response',
    'status',
    'submittedAtMs',
  ]);
}

describe('native reply/result process contract', () => {
  it(
    'submits exact file, stdin, and empty inline bodies and retrieves each result',
    { timeout: 30_000 },
    async () =>
      withSandbox(async (sandbox) => {
        await initializeSchema(sandbox);
        const fileBody = '\ufefffile\r\n\u0000日本語 😀\r\nRESPONSE-END-fake\n  ';
        const file = temporaryFile(sandbox, 'response.txt', Buffer.from(fileBody, 'utf8'));
        const fileSeed = seedResponse(sandbox.database, 'native-file');
        const fileResult = await runCli(sandbox, [
          'reply',
          fileSeed.requestId,
          '--receipt',
          fileSeed.v1Receipt,
          '--file',
          file,
          '--json',
        ]);
        const fileSubmission = assertSubmitted(fileResult, fileSeed);
        expect(parseWholeStdout(fileResult)).toMatchObject({
          bodyBytes: Buffer.byteLength(fileBody),
        });
        const fileRetrieved = await runCli(sandbox, ['result', fileSeed.requestId, '--json'], {
          outputLimitBytes: 2 * 1024 * 1024,
        });
        expectJsonSuccess(fileRetrieved, {
          status: 'completed',
          requestId: fileSeed.requestId,
          response: fileBody,
          bodyBytes: Buffer.byteLength(fileBody),
          submittedAtMs: fileSubmission.submittedAtMs,
        });

        const stdinBody = '\ufeffstdin\r\n\u0000日本語 😀\n';
        const stdinSeed = seedResponse(sandbox.database, 'native-stdin');
        const stdinResult = await runCli(
          sandbox,
          [
            'reply',
            stdinSeed.requestId,
            '--receipt',
            stdinSeed.compactReceipt,
            '--stdin',
            '--json',
          ],
          { stdin: Buffer.from(stdinBody, 'utf8'), outputLimitBytes: 2 * 1024 * 1024 }
        );
        const stdinSubmission = assertSubmitted(
          stdinResult,
          stdinSeed,
          Buffer.byteLength(stdinBody)
        );
        const stdinRetrieved = await runCli(sandbox, ['result', stdinSeed.requestId, '--json']);
        assertCompleted(stdinRetrieved, stdinSeed, stdinBody, stdinSubmission.submittedAtMs);

        const emptySeed = seedResponse(sandbox.database, 'native-empty');
        const emptyResult = await runCli(sandbox, [
          'reply',
          emptySeed.requestId,
          '--receipt',
          emptySeed.compactReceipt,
          '--message',
          '',
          '--json',
        ]);
        const emptySubmission = assertSubmitted(emptyResult, emptySeed);
        assertCompleted(
          await runCli(sandbox, ['result', emptySeed.requestId, '--json']),
          emptySeed,
          '',
          emptySubmission.submittedAtMs
        );
      })
  );

  it('accepts the one-megabyte stdin boundary without truncation', { timeout: 30_000 }, async () =>
    withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const seeded = seedResponse(sandbox.database, 'native-one-megabyte');
      const body = 'a'.repeat(MAX_RESPONSE_BYTES);
      const submitted = await runCli(
        sandbox,
        ['reply', seeded.requestId, '--receipt', seeded.compactReceipt, '--stdin', '--json'],
        { stdin: Buffer.from(body, 'utf8'), outputLimitBytes: 2 * 1024 * 1024 }
      );
      const submission = assertSubmitted(submitted, seeded, MAX_RESPONSE_BYTES);
      const result = await runCli(sandbox, ['result', seeded.requestId, '--json'], {
        outputLimitBytes: 2 * 1024 * 1024,
      });
      assertCompleted(result, seeded, body, submission.submittedAtMs);
    })
  );

  it(
    'keeps identical retries immutable and rejects conflicting bodies without mutation',
    { timeout: 30_000 },
    async () =>
      withSandbox(async (sandbox) => {
        await initializeSchema(sandbox);
        const seeded = seedResponse(sandbox.database, 'native-retry');
        const firstBody = 'original\r\n日本語';
        const first = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          seeded.v1Receipt,
          '--message',
          firstBody,
          '--json',
        ]);
        const firstDocument = assertSubmitted(first, seeded);
        const beforeConflict = responseSnapshot(sandbox.database, seeded.requestId);
        const retry = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          seeded.compactReceipt,
          '--message',
          firstBody,
          '--json',
        ]);
        assertSubmitted(retry, seeded, Buffer.byteLength(firstBody));
        expect(parseWholeStdout(retry)).toMatchObject({
          status: 'submitted',
          requestId: seeded.requestId,
          bodyBytes: Buffer.byteLength(firstBody),
          submittedAtMs: firstDocument.submittedAtMs,
        });
        const rejected = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          seeded.compactReceipt,
          '--message',
          'different body',
          '--json',
        ]);
        expect(rejected.status).toBe(5);
        expectError(rejected, 'RESPONSE_CONFLICT');
        expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(beforeConflict);
        removeAttempt(sandbox.database, seeded.requestId);
        const orphanRetry = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          seeded.compactReceipt,
          '--message',
          firstBody,
          '--json',
        ]);
        expect(assertSubmitted(orphanRetry, seeded).submittedAtMs).toBe(
          firstDocument.submittedAtMs
        );
        expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual({
          attempt: undefined,
          response: beforeConflict.response,
        });
        expectJsonSuccess(await runCli(sandbox, ['result', seeded.requestId, '--json']), {
          status: 'completed',
          requestId: seeded.requestId,
          response: firstBody,
          bodyBytes: Buffer.byteLength(firstBody),
          submittedAtMs: firstDocument.submittedAtMs,
        });
      })
  );

  it(
    'keeps storage-only reply/result independent from malformed config and tmux',
    { timeout: 30_000 },
    async () =>
      withSandbox(async (sandbox) => {
        await initializeSchema(sandbox);
        malformedConfig(sandbox);
        const tmuxLog = installTmuxTripwire(sandbox);
        const seeded = seedResponse(sandbox.database, 'native-storage-only');
        const body = 'storage-only exact result';
        const submitted = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          seeded.compactReceipt,
          '--message',
          body,
          '--json',
        ]);
        expect(submitted.status).toBe(0);
        expectJsonSuccess(await runCli(sandbox, ['result', seeded.requestId, '--json']), {
          status: 'completed',
          requestId: seeded.requestId,
          response: body,
          bodyBytes: Buffer.byteLength(body),
          submittedAtMs: (parseWholeStdout(submitted) as { submittedAtMs: number }).submittedAtMs,
        });
        expect(existsSync(tmuxLog)).toBe(false);
        expect(readFileSync(sandbox.globalConfig, 'utf8')).toBe('{ malformed global config');
        expect(readFileSync(sandbox.localConfig, 'utf8')).toBe('{ malformed local config');
      })
  );

  it('opens a stopped schema8 database through the native public reply path', async () =>
    withSandbox(async (sandbox) => {
      initializeHistoricalDatabase(sandbox.database);
      expect(schemaVersion(sandbox.database)).toBe(8);
      const seeded = seedResponse(sandbox.database, 'native-schema8-reply');
      const body = 'schema8 migration exact body';
      const submitted = await runCli(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.v1Receipt,
        '--message',
        body,
        '--json',
      ]);
      const submission = assertSubmitted(submitted, seeded, Buffer.byteLength(body));
      assertCompleted(
        await runCli(sandbox, ['result', seeded.requestId, '--json']),
        seeded,
        body,
        submission.submittedAtMs
      );
      expect(schemaVersion(sandbox.database)).toBe(EXPECTED_NATIVE_SCHEMA_VERSION);
    }));

  it(
    'reports unavailable results and sanitized errors for unknown, pending, and expired requests',
    { timeout: 30_000 },
    async () =>
      withSandbox(async (sandbox) => {
        await initializeSchema(sandbox);
        const pending = seedResponse(sandbox.database, 'native-pending', { status: 'prepared' });
        const expired = seedResponse(sandbox.database, 'native-expired', { expired: true });
        for (const requestId of [pending.requestId, expired.requestId, 'native-unknown']) {
          const result = await runCli(sandbox, ['result', requestId, '--json']);
          expect(result.status).toBe(3);
          expectError(result, 'RESPONSE_NOT_AVAILABLE');
          expect(parseWholeStdout(result)).toEqual({
            status: 'unavailable',
            requestId,
            error: { code: 'RESPONSE_NOT_AVAILABLE', message: expect.any(String) },
          });
        }
        const unknown = seedResponse(sandbox.database, 'native-unknown-reply');
        const unknownReceipt = v1Receipt('unknown-request', unknown.attemptId);
        const unknownResult = await runCli(sandbox, [
          'reply',
          'unknown-request',
          '--receipt',
          unknownReceipt,
          '--message',
          'body',
          '--json',
        ]);
        expect(unknownResult.status).toBe(3);
        expectError(unknownResult, 'RESPONSE_REQUEST_NOT_FOUND');
        const expiredReply = await runCli(sandbox, [
          'reply',
          expired.requestId,
          '--receipt',
          expired.compactReceipt,
          '--message',
          'too late',
          '--json',
        ]);
        expect(expiredReply.status).toBe(1);
        expectError(expiredReply, 'RESPONSE_EXPIRED');
        const failed = await runCli(sandbox, ['result', 'x'.repeat(257), '--json']);
        expect(failed.status).toBe(1);
        expectError(failed, 'USAGE_ERROR');
      })
  );

  it('rejects invalid file and stdin sources before finalization', { timeout: 30_000 }, async () =>
    withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const seeded = seedResponse(sandbox.database, 'native-invalid-input');
      const before = responseSnapshot(sandbox.database, seeded.requestId);
      const missing = await runCli(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        'not-a-receipt',
        '--file',
        path.join(sandbox.root, 'missing.txt'),
        '--json',
      ]);
      expect(missing.status).toBe(1);
      expectError(missing, 'RESPONSE_RECEIPT_INVALID');
      expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(before);

      const malformed = temporaryFile(sandbox, 'malformed.txt', Buffer.from([0xc3, 0x28]));
      const oversized = temporaryFile(
        sandbox,
        'oversized.txt',
        Buffer.concat([Buffer.alloc(MAX_RESPONSE_BYTES, 0x61), Buffer.from('x')])
      );
      for (const [file, code] of [
        [malformed, 'RESPONSE_INPUT_INVALID'],
        [oversized, 'RESPONSE_INPUT_TOO_LARGE'],
      ] as const) {
        const result = await runCli(sandbox, [
          'reply',
          seeded.requestId,
          '--receipt',
          seeded.v1Receipt,
          '--file',
          file,
          '--json',
        ]);
        expect(result.status).toBe(1);
        expectError(result, code);
        expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(before);
      }

      const directoryResult = await runCli(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.v1Receipt,
        '--file',
        sandbox.root,
        '--json',
      ]);
      expect(directoryResult.status).toBe(1);
      expectError(directoryResult, 'RESPONSE_FILE_ERROR');

      const fifo = path.join(sandbox.root, 'response.fifo');
      execFileSync('mkfifo', [fifo], { timeout: 5_000 });
      const fifoResult = await runCli(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.v1Receipt,
        '--file',
        fifo,
        '--json',
      ]);
      expect(fifoResult.status).toBe(1);
      expectError(fifoResult, 'RESPONSE_FILE_ERROR');

      for (const [input, code] of [
        [Buffer.from([0xc3, 0x28]), 'RESPONSE_INPUT_INVALID'],
        [
          Buffer.concat([Buffer.alloc(MAX_RESPONSE_BYTES, 0x61), Buffer.from('x')]),
          'RESPONSE_INPUT_TOO_LARGE',
        ],
      ] as const) {
        const result = await runCli(
          sandbox,
          ['reply', seeded.requestId, '--receipt', seeded.v1Receipt, '--stdin', '--json'],
          { stdin: input, outputLimitBytes: 2 * 1024 * 1024 }
        );
        expect(result.status).toBe(1);
        expectError(result, code);
        expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(before);
      }
    })
  );

  it(
    'maps an open stdin past the five-second input deadline without finalizing',
    { timeout: 15_000 },
    async () =>
      withSandbox(async (sandbox) => {
        await initializeSchema(sandbox);
        const seeded = seedResponse(sandbox.database, 'native-stdin-timeout');
        const before = responseSnapshot(sandbox.database, seeded.requestId);
        const result = await runCli(
          sandbox,
          ['reply', seeded.requestId, '--receipt', seeded.v1Receipt, '--stdin', '--json'],
          { stdin: Buffer.from('partial'), closeStdin: false, deadlineMs: 8_000 }
        );
        expect(result.status).toBe(4);
        expectError(result, 'RESPONSE_INPUT_TIMEOUT');
        expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(before);
      })
  );

  it(
    'rejects input before creating storage and validates receipts before reading stdin',
    { timeout: 15_000 },
    async () =>
      withSandbox(async (sandbox) => {
        const requestId = 'not-yet-open';
        const receipt = v1Receipt(requestId, 'attempt');
        const malformed = temporaryFile(sandbox, 'bad-body', Buffer.from([0xff]));
        for (const [input, code] of [
          [['--file', malformed], 'RESPONSE_INPUT_INVALID'],
          [['--file', path.join(sandbox.root, 'missing')], 'RESPONSE_FILE_ERROR'],
        ] as const) {
          const result = await runCli(sandbox, [
            'reply',
            requestId,
            '--receipt',
            receipt,
            ...input,
            '--json',
          ]);
          expect(result.status).toBe(1);
          expectError(result, code);
          expect(existsSync(sandbox.database)).toBe(false);
        }
        const rejected = await runCli(
          sandbox,
          ['reply', requestId, '--receipt', 'malformed', '--stdin', '--json'],
          { stdin: 'partial', closeStdin: false }
        );
        expect(rejected.status).toBe(1);
        expectError(rejected, 'RESPONSE_RECEIPT_INVALID');
        expect(existsSync(sandbox.database)).toBe(false);
        const timeout = await runCli(
          sandbox,
          ['reply', requestId, '--receipt', receipt, '--stdin', '--json'],
          { stdin: 'partial', closeStdin: false, deadlineMs: 8_000 }
        );
        expect(timeout.status).toBe(4);
        expectError(timeout, 'RESPONSE_INPUT_TIMEOUT');
        expect(existsSync(sandbox.database)).toBe(false);
      })
  );

  it('rejects a well-formed compact proof for another request without mutations', async () =>
    withSandbox(async (sandbox) => {
      await initializeSchema(sandbox);
      const target = seedResponse(sandbox.database, 'target');
      const other = seedResponse(sandbox.database, 'other');
      const before = responseSnapshot(sandbox.database, target.requestId);
      const result = await runCli(sandbox, [
        'reply',
        target.requestId,
        '--receipt',
        other.compactReceipt,
        '--message',
        'not authorized by this receipt',
        '--json',
      ]);
      expect(result.status).toBe(1);
      expectError(result, 'RESPONSE_RECEIPT_MISMATCH');
      expect(responseSnapshot(sandbox.database, target.requestId)).toEqual(before);
    }));
});
