import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';

interface RoomResult {
  room: { id: string };
}
interface DispatchResult {
  items: { requestId: string; acceptance: string }[];
}
interface TalkResult {
  requestId: string;
  status: string;
  response: string;
}

describe('shared rooms across real tmux and inbox delivery', { concurrent: false }, () => {
  it('preserves verified caller provenance and unknown outside-owner provenance', async () => {
    await withE2EFixture(async (fixture) => {
      const caller = expectJsonResult(
        await fixture.runJsonCli<{ id: string; bound: boolean }>(['name', 'Caller', '-s'])
      );
      expect(caller).toMatchObject({ id: expect.stringMatching(/^[0-9a-f-]{36}$/), bound: true });
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Receiver']));
      const { room } = expectJsonResult(
        await fixture.runJsonCli<RoomResult>(['room', 'create', 'Design'])
      );
      expectJsonResult(
        await fixture.runJsonCli(['room', 'join', room.id, '--identity', 'Receiver'])
      );
      const verified = expectJsonResult(
        await fixture.runJsonCli<DispatchResult>(['room', 'send', room.id, 'From a bound caller'])
      );
      const owner = expectJsonResult(
        await fixture.runJsonCli<DispatchResult>(
          ['room', 'send', room.id, 'From an outside owner'],
          { outsideTmux: true }
        )
      );
      for (const receipt of [verified, owner]) {
        expect(receipt.items).toHaveLength(1);
        expect(receipt.items[0]?.acceptance).toBe('queued');
      }
      const attempts = requestAttempts(fixture);
      expect(attempts).toHaveLength(2);
      expect(attempts.find((row) => row.request_id === verified.items[0]?.requestId)).toMatchObject(
        {
          originator_kind: 'verified',
          originator_identity_id: caller.id,
          room_id: room.id,
          status: 'queued',
          message_text: 'From a bound caller',
        }
      );
      expect(attempts.find((row) => row.request_id === owner.items[0]?.requestId)).toMatchObject({
        originator_kind: 'unknown',
        originator_identity_id: null,
        room_id: room.id,
        status: 'queued',
        message_text: 'From an outside owner',
      });
      expect(fixture.events().filter((event) => event.event === 'request')).toEqual([]);
    });
  });

  it('sends a room-scoped pane request to one member and rejects another bound pane', async () => {
    await withE2EFixture(async (fixture) => {
      const member = await fixture.createMockPane('member');
      const outsider = await fixture.createMockPane('outsider');
      expectJsonResult(await fixture.runJsonCli(['name', 'Caller', '-s']));
      expectJsonResult(await fixture.runJsonCli(['add', member.pane, 'Member']));
      expectJsonResult(await fixture.runJsonCli(['add', outsider.pane, 'Outside']));
      const { room } = expectJsonResult(
        await fixture.runJsonCli<RoomResult>(['room', 'create', 'Design'])
      );
      expectJsonResult(await fixture.runJsonCli(['room', 'join', room.id, '--identity', 'Member']));
      const options = ['--room', room.id, '--no-preamble', '--timeout', '8'];
      const completed = expectJsonResult(
        await fixture.runJsonCli<TalkResult>([
          'talk',
          'Member',
          'Review the room drawing',
          ...options,
        ])
      );
      expect(completed).toMatchObject({
        status: 'completed',
        response: 'mock-agent response: Review the room drawing',
      });
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.requestId === completed.requestId &&
          event.pid === member.pid
      );
      expect(requestAttempts(fixture)).toMatchObject([
        {
          request_id: completed.requestId,
          room_id: room.id,
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
        },
      ]);
      const rejected = await fixture.runJsonCli<{ error: { code: string } }>([
        'talk',
        'Outside',
        'Must not be delivered',
        ...options,
      ]);
      expect(rejected.code).toBe(1);
      expect(rejected.json?.error.code).toBe('ROOM_RECIPIENT_NOT_MEMBER');
      expect(requestAttempts(fixture)).toHaveLength(1);
      expect(fixture.events().filter((event) => event.event === 'request')).toMatchObject([
        { requestId: completed.requestId, pid: member.pid },
      ]);
    });
  });
});
