import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeExecutable } from '../support/executable-fixture.mjs';
import { randomUUID } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { assertRemoteDeviceVectors, object, text } from '../support/remote-device.js';
import { dispatchIntent, RemoteOwner } from '../support/remote-owner.js';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { requestAttempts, requestResponses } from './request-state-oracle.js';
import { installTmuxTrace } from './tmux-trace.js';

async function name(fixture: E2EFixture, value: string, pane = fixture.pane): Promise<string> {
  return expectJsonResult(
    await fixture.runJsonCli<{ id: string }>(['name', value, '--save'], { pane })
  ).id;
}
async function queuedWake(
  fixture: E2EFixture,
  requestId: string,
  recipientId: string,
  preview: string
): Promise<void> {
  const wake = await fixture.waitForEvent(
    (event) => event.event === 'input' && event.line?.includes(requestId) === true
  );
  expect(wake.line).toBe(
    `▚ ◆ anonymous · ${preview} · tmt x show ${requestId} --incoming --identity ${recipientId} --json`
  );
}
const accepted = (operationId: string, requestId: string) => ({
  state: 'accepted',
  operationId,
  requestId,
});
// #1055's six acceptance bullets plus the single owner-device reads scenario.
// All application actions go through the real Remote and selected core binaries.
describe('Remote owner-device operations (#1055)', () => {
  it('bullets 1/4: delivers direct intent once, recovers its receipt and refuses changed intent', async () => {
    await withE2EFixture(
      async (fixture) => {
        const id = await name(fixture, 'remote-direct');
        const owner = new RemoteOwner(fixture);
        try {
          await owner.start();
          const { device, paired } = await owner.pair(undefined, { talk: true });
          const session = await owner.session(device, paired);
          const operationId = randomUUID();
          const message = 'Direct Remote acceptance';
          const before = requestAttempts(fixture);
          const intent = dispatchIntent(operationId, id, message);
          const sent = await session.append('dispatch.create', intent, operationId);
          const requestId = text(sent.requestId);
          expect(sent).toEqual(accepted(operationId, requestId));
          await queuedWake(
            fixture,
            requestId,
            id,
            '[remote: E2E owner device] Direct Remote accepta…'
          );
          const rows = requestAttempts(fixture).filter((row) => row.request_id === requestId);
          expect(rows).toHaveLength(1);
          expect(rows[0]).toMatchObject({
            recipient_identity_id: id,
            message_text: `[remote: ${device.name}]\n${message}`,
          });
          expect(requestAttempts(fixture)).toHaveLength(before.length + 1);
          const settled = requestAttempts(fixture);
          const events = fixture.events();
          expect(await session.append('dispatch.create', intent, operationId)).toEqual(sent);
          expect(await session.append('operation.show', { operationId })).toEqual(sent);
          expect(
            await session.append(
              'dispatch.create',
              dispatchIntent(operationId, id, 'Changed intent'),
              operationId
            )
          ).toMatchObject({ error: { code: 'REMOTE_INTENT_CONFLICT' } });
          expect(
            owner.coreCalls().filter((call) => call.operation === 'dispatch.create')
          ).toHaveLength(1);
          expect(requestAttempts(fixture)).toEqual(settled);
          expect(fixture.events()).toEqual(events);
        } finally {
          await owner.dispose();
        }
      },
      { mode: 'input-log' }
    );
  }, 30_000);

  it('bullet 2: held intent has no core effect until explicit local confirmation; refusal/EOF/cancel never dispatch', async () => {
    await withE2EFixture(
      async (fixture) => {
        const id = await name(fixture, 'remote-held');
        const owner = new RemoteOwner(fixture);
        try {
          await owner.start();
          const { device, paired } = await owner.pair(undefined, { talk: true });
          await owner.stop();
          owner.seedGrant(paired.clientId, { mode: 'hold' });
          await owner.start();
          const session = await owner.session(device, paired);
          const before = requestAttempts(fixture);
          const eventsBefore = fixture.events();
          const effectCount = () =>
            owner.coreCalls().filter((call) => call.operation === 'dispatch.create').length;
          const operationId = randomUUID();
          const message = 'Owner must see this exact frozen intent';
          expect(
            await session.append(
              'dispatch.create',
              dispatchIntent(operationId, id, message),
              operationId
            )
          ).toEqual({ state: 'held', operationId });
          const approval = await owner.approval(operationId);
          expect(approval.held).toEqual({
            event: 'held',
            operationId,
            clientId: paired.clientId,
            deviceName: device.name,
            recipientId: id,
            message: `[remote: ${device.name}]\n${message}`,
          });
          expect(effectCount()).toBe(0);
          expect(requestAttempts(fixture)).toEqual(before);
          expect(fixture.events()).toEqual(eventsBefore);
          const ended = await approval.finish('confirm');
          const requestId = text(ended.requestId);
          expect(ended).toEqual({ event: 'ended', ...accepted(operationId, requestId) });
          await queuedWake(
            fixture,
            requestId,
            id,
            '[remote: E2E owner device] Owner must see this e…'
          );
          expect(requestAttempts(fixture).filter((row) => row.request_id === requestId)).toEqual([
            expect.objectContaining({
              message_text: approval.held.message,
              recipient_identity_id: id,
            }),
          ]);
          expect(effectCount()).toBe(1);
          const settled = requestAttempts(fixture);
          const settledEvents = fixture.events();
          for (const answer of ['refuse', 'eof', 'cancel'] as const) {
            const cancelledId = randomUUID();
            expect(
              await session.append(
                'dispatch.create',
                dispatchIntent(cancelledId, id, `Must cancel ${answer}`),
                cancelledId
              )
            ).toEqual({ state: 'held', operationId: cancelledId });
            if (answer === 'cancel')
              expect(await owner.cancel(cancelledId)).toEqual({
                state: 'cancelled',
                operationId: cancelledId,
              });
            else {
              const local = await owner.approval(cancelledId);
              expect(await local.finish(answer)).toEqual({
                event: 'ended',
                state: 'cancelled',
                operationId: cancelledId,
              });
            }
            expect(await session.append('operation.show', { operationId: cancelledId })).toEqual({
              state: 'cancelled',
              operationId: cancelledId,
            });
          }
          expect(effectCount()).toBe(1);
          expect(requestAttempts(fixture)).toEqual(settled);
          expect(fixture.events()).toEqual(settledEvents);
        } finally {
          await owner.dispose();
        }
      },
      { mode: 'input-log' }
    );
  }, 45_000);

  it('bullet 3: permits its selected identity, refuses another identity and refuses an expired grant', async () => {
    await withE2EFixture(
      async (fixture) => {
        const allowed = await name(fixture, 'remote-allowed');
        const otherPane = await fixture.createMockPane('remote-denied');
        const denied = await name(fixture, 'remote-denied', otherPane.pane);
        const owner = new RemoteOwner(fixture);
        try {
          await owner.start();
          const { device, paired } = await owner.pair(undefined, { talk: true });
          await owner.stop();
          owner.seedGrant(paired.clientId, { agents: [allowed] });
          await owner.start();
          const session = await owner.session(device, paired);
          const allowedId = randomUUID();
          const sent = await session.append(
            'dispatch.create',
            dispatchIntent(allowedId, allowed, 'Allowed control'),
            allowedId
          );
          const requestId = text(sent.requestId);
          expect(sent).toEqual(accepted(allowedId, requestId));
          await queuedWake(
            fixture,
            requestId,
            allowed,
            '[remote: E2E owner device] Allowed control'
          );
          const rows = requestAttempts(fixture);
          const events = fixture.events();
          const effects = owner.coreCalls().filter((call) => call.operation === 'dispatch.create');
          const deniedId = randomUUID();
          expect(
            await session.append(
              'dispatch.create',
              dispatchIntent(deniedId, denied, 'Forbidden identity'),
              deniedId
            )
          ).toMatchObject({ error: { code: 'REMOTE_SCOPE_DENIED' } });
          expect(owner.coreCalls().filter((call) => call.operation === 'dispatch.create')).toEqual(
            effects
          );
          expect(requestAttempts(fixture)).toEqual(rows);
          expect(fixture.events()).toEqual(events);
          await owner.stop();
          owner.seedGrant(paired.clientId, { expiresAtMs: Date.now() - 1 });
          await owner.start();
          // Fresh window, nonce, signature and timestamp: expiry is the only
          // changed authority, not a stale session or replayed request.
          const expired = await owner.post('append', owner.opening(device, paired));
          expect(expired.status).toBe(404);
          expect(expired.rawBody).toBe('{}');
          expect(expired.body).toEqual({});
          expect(expired.headers['set-cookie']).toBeUndefined();
          expect(owner.coreCalls().filter((call) => call.operation === 'dispatch.create')).toEqual(
            effects
          );
          expect(requestAttempts(fixture)).toEqual(rows);
          expect(fixture.events()).toEqual(events);
        } finally {
          await owner.dispose();
        }
      },
      { mode: 'input-log' }
    );
  }, 40_000);

  it('owner-device reads: permitted projections and pending/empty final result work; out-of-scope read refuses', async () => {
    await withE2EFixture(
      async (fixture) => {
        const allowed = await name(fixture, 'remote-read');
        const other = await fixture.createMockPane('remote-read-denied');
        const denied = await name(fixture, 'remote-read-denied', other.pane);
        const owner = new RemoteOwner(fixture);
        try {
          await owner.start();
          const { device, paired } = await owner.pair(undefined, { talk: true });
          await owner.stop();
          owner.seedGrant(paired.clientId, { agents: [allowed] });
          await owner.start();
          const session = await owner.session(device, paired);
          const operationId = randomUUID();
          const sent = await session.append(
            'dispatch.create',
            dispatchIntent(operationId, allowed, 'Read causal control'),
            operationId
          );
          const requestId = text(sent.requestId);
          expect(sent).toEqual(accepted(operationId, requestId));
          await queuedWake(
            fixture,
            requestId,
            allowed,
            '[remote: E2E owner device] Read causal control'
          );
          const count = owner
            .coreCalls()
            .filter((call) => call.operation === 'dispatch.create').length;
          const directory = await session.append('agents.list', {});
          expect(directory.identities).toEqual([
            expect.objectContaining({ id: allowed, name: 'remote-read' }),
          ]);
          for (const entry of directory.identities as unknown[]) {
            expect(
              Object.keys(object(entry)).every((key) =>
                ['id', 'name', 'presence', 'delivery'].includes(key)
              )
            ).toBe(true);
          }
          expect(
            await session.append('identities.status', {
              version: 1,
              operation: 'identities.status',
              input: { identityIds: [allowed] },
            })
          ).toMatchObject({ identities: [{ id: allowed, found: true }] });
          const check = await session.append('check', { agentId: allowed, lines: 10 });
          const localCheck = expectJsonResult(
            await fixture.runJsonCli(['check', 'remote-read', '--lines', '10'])
          );
          expect(check).toMatchObject({ target: 'remote-read', lines: 10 });
          expect(check).toEqual(localCheck);
          expect(await session.append('result', { requestId })).toEqual({
            state: 'pending',
            requestId,
          });
          expect(
            await session.append('requests.show', {
              version: 1,
              operation: 'requests.show',
              input: { requestId },
            })
          ).toMatchObject({ requestId, recipientId: allowed, final: { status: 'not_submitted' } });
          // The core queues an inbox request and sends one advisory wake; it
          // does not paste a talk reply frame. Act as the recipient through the
          // actual public receipt and CLI, without fabricating a final response.
          const incoming = expectJsonResult(
            await fixture.runJsonCli<{
              exchange: { prompt: { message: string }; reply: { receipt: string } };
            }>(['x', 'show', requestId, '--incoming', '--identity', allowed], { withoutTmux: true })
          );
          expect(incoming.exchange.prompt.message).toBe(
            `[remote: ${device.name}]\nRead causal control`
          );
          expect(incoming.exchange.reply.receipt).toMatch(/^v2_/);
          expect(
            expectJsonResult(
              await fixture.runJsonCli(
                ['reply', requestId, '--receipt', incoming.exchange.reply.receipt, '--message', ''],
                { withoutTmux: true }
              )
            )
          ).toMatchObject({ status: 'submitted', requestId, bodyBytes: 0 });
          expect(await session.append('result', { requestId })).toEqual({
            state: 'replied',
            requestId,
            message: '',
          });
          expect(
            await session.append('requests.show', {
              version: 1,
              operation: 'requests.show',
              input: { requestId },
            })
          ).toMatchObject({ requestId, final: { status: 'retained', response: '', bodyBytes: 0 } });
          expect(requestResponses(fixture)).toContainEqual(
            expect.objectContaining({ request_id: requestId, body: '' })
          );
          const missing = `req_${randomUUID()}`;
          expect(await session.append('result', { requestId: missing })).toMatchObject({
            state: 'unavailable',
            requestId: missing,
          });
          expect(await session.append('operation.show', { operationId })).toEqual(sent);
          expect(
            await session.append('dispatch.show', {
              version: 1,
              operation: 'dispatch.show',
              input: { operationId },
            })
          ).toMatchObject({
            operationId,
            items: [expect.objectContaining({ recipientId: allowed, requestId })],
          });
          const rows = requestAttempts(fixture);
          const events = fixture.events();
          const calls = owner.coreCalls();
          expect(await session.append('check', { agentId: denied, lines: 10 })).toMatchObject({
            error: { code: 'REMOTE_SCOPE_DENIED' },
          });
          expect(owner.coreCalls()).toEqual(calls);
          expect(
            owner.coreCalls().filter((call) => call.operation === 'dispatch.create')
          ).toHaveLength(count);
          expect(requestAttempts(fixture)).toEqual(rows);
          expect(fixture.events()).toEqual(events);
        } finally {
          await owner.dispose();
        }
      },
      { mode: 'input-log' }
    );
  }, 45_000);

  it('bullet 6: an enrolled pane receives Remote through its channel and is never pasted to', async () => {
    await withE2EFixture(async (fixture) => {
      const pane = fixture.createShellPane('remote-channel').pane;
      const log = path.join(fixture.root, 'remote-channel.log');
      const status = path.join(fixture.root, 'remote-channel.status');
      const home = path.join(fixture.root, 'remote-channel-home');
      fs.mkdirSync(path.join(home, '.claude'), { recursive: true });
      const mock = fileURLToPath(new URL('./mock-claude-channel.mjs', import.meta.url));
      const launcher = path.join(fixture.wrapperDir, 'claude');
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      writeExecutable(launcher, `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(mock)} "$@"\n`);
      const command = [
        'env',
        `HOME=${home}`,
        `MOCK_CHANNEL_LOG=${log}`,
        `MOCK_DB=${path.join(fixture.globalDir, 'tmux-team.db')}`,
        'MOCK_REQUEST_ON_WAKE=1',
        fixture.executables.cli.executable,
        ...fixture.executables.cli.args,
        'run',
        '--channel',
        '-s',
        'remote-channel',
        launcher,
      ]
        .map(quote)
        .join(' ');
      fixture.tmux([
        'send-keys',
        '-t',
        pane,
        '-l',
        `${command}; printf '%s' "$?" > ${quote(status)}`,
      ]);
      fixture.tmux(['send-keys', '-t', pane, 'Enter']);
      const events = (): Array<Record<string, unknown>> =>
        fs.existsSync(log)
          ? fs
              .readFileSync(log, 'utf8')
              .split('\n')
              .filter(Boolean)
              .map((line) => object(JSON.parse(line)))
          : [];
      const owner = new RemoteOwner(fixture);
      const trace = installTmuxTrace(fixture);
      const errors: unknown[] = [];
      try {
        await fixture.waitFor(
          () => events().some((e) => e.event === 'initialized-sent'),
          15_000,
          'existing mock channel handshake'
        );
        await fixture.waitFor(
          () => {
            const directory = path.join(fixture.globalDir, 'channels');
            if (!fs.existsSync(directory)) return false;
            return fs
              .readdirSync(directory)
              .filter((file) => file.endsWith('.json'))
              .some(
                (file) =>
                  object(JSON.parse(fs.readFileSync(path.join(directory, file), 'utf8'))).claude !==
                  null
              );
          },
          15_000,
          'actual core channel enrollment published'
        );
        const identity = expectJsonResult(
          await fixture.runJsonCli<{ id: string }>(['whoami'], { pane })
        );
        const id = identity.id;
        await owner.start();
        const { device, paired } = await owner.pair(undefined, { talk: true });
        const session = await owner.session(device, paired);
        const operationId = randomUUID();
        const message = 'Remote channel delivery';
        trace.clear();
        const sent = await session.append(
          'dispatch.create',
          dispatchIntent(operationId, id, message),
          operationId
        );
        const requestId = text(sent.requestId);
        expect(sent).toEqual(accepted(operationId, requestId));
        await fixture.waitFor(
          () => events().some((e) => e.event === 'wake-request' && e.requestId === requestId),
          15_000,
          'channel peer reads real durable inbox'
        );
        expect(
          events().filter((e) => e.event === 'wake-request' && e.requestId === requestId)
        ).toEqual([
          expect.objectContaining({ messageText: `[remote: ${device.name}]\n${message}` }),
        ]);
        expect(
          await session.append(
            'dispatch.create',
            dispatchIntent(operationId, id, message),
            operationId
          )
        ).toEqual(sent);
        expect(requestAttempts(fixture).filter((row) => row.request_id === requestId)).toEqual([
          expect.objectContaining({
            route_kind: 'inbox',
            status: 'queued',
            recipient_identity_id: id,
          }),
        ]);
        expect(events().filter((e) => e.event === 'channel')).toHaveLength(1);
        expect(events().filter((e) => e.event === 'paste')).toEqual([]);
        expect(trace.commands()).not.toContain('paste-buffer');
        expect(trace.commands()).not.toContain('send-keys');
      } catch (error) {
        errors.push(error);
      } finally {
        try {
          await owner.dispose();
        } catch (error) {
          errors.push(error);
        }
        try {
          fs.writeFileSync(`${log}.quit`, '');
          await fixture.waitFor(
            () => fs.existsSync(status) && fs.readFileSync(status, 'utf8') === '0',
            15_000,
            'owned channel runtime joined'
          );
          await fixture.waitFor(
            () =>
              fs
                .readdirSync(path.join(fixture.globalDir, 'channels'))
                .filter((file) => file !== '.lock').length === 0,
            5000,
            'channel enrollment/socket removed'
          );
        } catch (error) {
          errors.push(error);
        }
      }
      if (errors.length)
        throw new AggregateError(errors, 'Remote/channel scenario or joined teardown failed');
    });
  }, 60_000);

  it('bullet 4: refuses replay and out-of-order dispatch before core or pane effects', async () => {
    assertRemoteDeviceVectors();
    await withE2EFixture(async (fixture) => {
      const identity = expectJsonResult(
        await fixture.runJsonCli<{ id: string }>(['name', 'remote-sequence-agent', '--save'])
      );
      const owner = new RemoteOwner(fixture);
      const trace = installTmuxTrace(fixture);
      try {
        await owner.start();
        const { device, paired } = await owner.pair(undefined, { talk: true });
        const session = await owner.session(device, paired);
        const before = requestAttempts(fixture);
        const coreBefore = owner.coreCalls();
        trace.clear();
        const eventsBefore = fixture.events();
        const operationId = randomUUID();
        const dispatch = {
          version: 1,
          operation: 'dispatch.create',
          originator: 'anonymous',
          input: {
            operationId,
            recipientIds: [identity.id],
            message: 'This out-of-order intent must never reach the agent',
            kind: 'request',
          },
        };
        const future = session.envelope('dispatch.create', dispatch, {
          id: operationId,
          sequence: '2',
        });
        expect(await session.exchange('append', future)).toMatchObject({
          error: { code: 'REMOTE_REPLAY' },
        });

        // The expected sequence remains usable: an independently verified
        // machine response and empty cursor are the positive wire control.
        const subscribe = session.envelope(
          'subscribe',
          { cursor: null, limit: 1, waitMs: 0 },
          { kind: 'control' }
        );
        const page = await session.exchange('subscribe', subscribe);
        expect(page).toMatchObject({ entries: [], reason: 'timeout', hasMore: false });
        const cursor = text(page.nextCursor);
        expect(cursor.length).toBeGreaterThan(0);
        expect(await session.exchange('subscribe', subscribe)).toMatchObject({
          error: { code: 'REMOTE_REPLAY' },
        });
        const stale = session.envelope('dispatch.create', dispatch, {
          id: operationId,
          sequence: '1',
        });
        expect(await session.exchange('append', stale)).toMatchObject({
          error: { code: 'REMOTE_REPLAY' },
        });

        const ack = session.envelope('ack', { cursor }, { kind: 'control' });
        const acknowledged = await session.exchange('ack', ack);
        expect(acknowledged).toEqual({ cursor });
        const after = await session.exchange(
          'subscribe',
          session.envelope('subscribe', { cursor, limit: 1, waitMs: 0 }, { kind: 'control' })
        );
        expect(after).toMatchObject({
          entries: [],
          nextCursor: cursor,
          reason: 'timeout',
          hasMore: false,
        });
        expect(requestAttempts(fixture)).toEqual(before);
        expect(owner.coreCalls()).toEqual(coreBefore);
        expect(fixture.events()).toEqual(eventsBefore);
        expect(trace.commands()).not.toContain('paste-buffer');
        expect(trace.commands()).not.toContain('send-keys');
      } finally {
        await owner.dispose();
      }
    });
  }, 30_000);
});
