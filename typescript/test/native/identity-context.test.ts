import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire, installTmuxTripwire } from './tmux-tripwire.js';

describe('required identity preflight', () => {
  it.each(
    [
      ['role', 'show'],
      ['role', 'set', 'must not write'],
      ['role', 'clear'],
      ['identity', 'show'],
      ['x', 'list'],
      ['x', 'show', 'missing-request'],
      ['x', 'ack', 'missing-request', '--revision', '1'],
      ['x', 'ackall'],
    ].map((args) => ({ label: args.join(' '), args }))
  )('rejects $label without initializing storage for an outside caller', async ({ args }) => {
    await withSandbox(async (sandbox) => {
      installTmuxTripwire(sandbox);
      expect(existsSync(sandbox.database)).toBe(false);
      const result = await runCli(sandbox, [...args, '--json']);
      expect(result.status).toBe(1);
      expectError(result, 'IDENTITY_REQUIRED');
      if (args.join(' ') === 'identity show') {
        expect(result.stdout).toContain('identity show <name>');
        expect(result.stdout).not.toContain('--identity');
        const human = await runCli(sandbox, args);
        expect(human.status).toBe(1);
        expect(human.stdout).toBe('');
        expect(human.stderr).toBe(
          'An identity is required; use identity show <name> or run from a verified bound pane.\n'
        );
      }
      expect(existsSync(sandbox.database)).toBe(false);
      expect(existsSync(`${sandbox.database}-wal`)).toBe(false);
      expect(existsSync(`${sandbox.database}-shm`)).toBe(false);
    });
  });
});

describe('explicit identity selector', () => {
  it('prefers an active canonical UUID, retains normalized names, and never resurrects retirement', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = await calibrateTmuxTripwire(sandbox);
      const create = async (name: string) => {
        const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
        expect(result.status).toBe(0);
        return (parseWholeStdout(result) as { identity: { id: string } }).identity;
      };
      const selected = async (value: string) => {
        const result = await runCli(sandbox, ['x', 'list', '--identity', value, '--json']);
        expect(result.status, result.stdout + result.stderr).toBe(0);
        return (parseWholeStdout(result) as { identity: { id: string } }).identity.id;
      };
      const shown = async (value: string) => {
        const result = await runCli(sandbox, ['identity', 'show', value, '--json']);
        expect(result.status, result.stdout + result.stderr).toBe(0);
        return (parseWholeStdout(result) as { identity: { id: string } }).identity.id;
      };
      const receiver = await create('Reviewer');
      expect(await selected(receiver.id)).toBe(receiver.id);
      expect(await shown(receiver.id)).toBe(receiver.id);
      expect(await selected('rEvIeWeR')).toBe(receiver.id);
      expect(await shown('rEvIeWeR')).toBe(receiver.id);
      const collision = await create(receiver.id);
      expect(collision.id).not.toBe(receiver.id);
      expect(await selected(receiver.id)).toBe(receiver.id);
      expect(await shown(receiver.id)).toBe(receiver.id);

      const uuidShapedName = '11111111-1111-4111-8111-111111111111';
      const named = await create(uuidShapedName);
      expect(await selected(uuidShapedName)).toBe(named.id);
      expect(await shown(uuidShapedName)).toBe(named.id);
      const retired = await create('Retired');
      const removed = await runCli(sandbox, ['rm', 'Retired', '--force', '--json']);
      expect(removed.status).toBe(0);
      for (const value of [retired.id, '22222222-2222-4222-8222-222222222222']) {
        const missing = await runCli(sandbox, ['x', 'list', '--identity', value, '--json']);
        expect(missing.status).toBe(3);
        expectError(missing, 'NAME_NOT_FOUND');
        const missingShow = await runCli(sandbox, ['identity', 'show', value, '--json']);
        expect(missingShow.status).toBe(3);
        expectError(missingShow, 'NAME_NOT_FOUND');
      }
      const invalidShow = await runCli(sandbox, ['identity', 'show', '   ', '--json']);
      expect(invalidShow.status).toBe(1);
      expectError(invalidShow, 'INVALID_NAME');
      expect(readFileSync(tmuxLog, 'utf8')).toBe('\n');
    });
  });

  it('uses explicit UUID to inspect only its own incoming request without ambient TMUX', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = await calibrateTmuxTripwire(sandbox);
      const create = async (name: string) => {
        const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
        expect(result.status).toBe(0);
        return (parseWholeStdout(result) as { identity: { id: string } }).identity;
      };
      await create('Sender');
      const receiver = await create('Receiver');
      const foreign = await create('Foreign');
      const collision = await create(receiver.id);
      expect(collision.id).not.toBe(receiver.id);
      const sent = await runCli(sandbox, [
        'talk',
        'Receiver',
        'Only the recipient may read this.',
        '--inbox',
        '--identity',
        'Sender',
        '--detach',
        '--json',
      ]);
      expect(sent.status).toBe(0);
      const requestId = (parseWholeStdout(sent) as { requestId: string }).requestId;
      const args = (identity: string) => [
        'x',
        'show',
        requestId,
        '--incoming',
        '--identity',
        identity,
        '--json',
      ];
      const detail = await runCli(sandbox, args(receiver.id));
      expect(detail.status, detail.stdout + detail.stderr).toBe(0);
      expect(parseWholeStdout(detail)).toMatchObject({
        identity: { id: receiver.id },
        exchange: { requestId, prompt: { message: 'Only the recipient may read this.' } },
      });
      for (const wrongIdentity of [foreign.id, collision.id]) {
        const denied = await runCli(sandbox, args(wrongIdentity));
        expect(denied.status).toBe(3);
        expectError(denied, 'X_NOT_FOUND');
      }
      const uuidShapedName = '11111111-1111-4111-8111-111111111111';
      const fallback = await create(uuidShapedName);
      const fallbackSent = await runCli(sandbox, [
        'talk',
        uuidShapedName,
        'Fallback name owns only this request.',
        '--inbox',
        '--identity',
        'Sender',
        '--detach',
        '--json',
      ]);
      expect(fallbackSent.status).toBe(0);
      const fallbackRequest = (parseWholeStdout(fallbackSent) as { requestId: string }).requestId;
      const fallbackDetail = await runCli(sandbox, [
        'x',
        'show',
        fallbackRequest,
        '--incoming',
        '--identity',
        uuidShapedName,
        '--json',
      ]);
      expect(fallbackDetail.status).toBe(0);
      expect(parseWholeStdout(fallbackDetail)).toMatchObject({
        identity: { id: fallback.id },
        exchange: {
          requestId: fallbackRequest,
          prompt: { message: 'Fallback name owns only this request.' },
        },
      });
      const fallbackCannotReadReceiver = await runCli(sandbox, args(uuidShapedName));
      expect(fallbackCannotReadReceiver.status).toBe(3);
      expectError(fallbackCannotReadReceiver, 'X_NOT_FOUND');
      expect(readFileSync(tmuxLog, 'utf8')).toBe('\n');
      const implicit = await runCli(sandbox, ['x', 'list', '--json']);
      expect(implicit.status).toBe(1);
      expectError(implicit, 'IDENTITY_REQUIRED');
    });
  });
});
