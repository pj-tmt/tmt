import { randomUUID } from 'node:crypto';
import { readFileSync, realpathSync, symlinkSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { createArtifact, type ArtifactFixture } from '../support/native-artifact.js';
import { dispatchIntent, RemoteOwner } from '../support/remote-owner.js';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';

const restart =
  'Remote was upgraded while its door is running. Finish active pairing and held approvals, then restart: tmt remote stop && tmt remote serve.';

describe('post-upgrade Remote notice', () => {
  it('replaces real release bytes, hints once and preserves the door, paired device and held approval', async () => {
    await withE2EFixture(
      async (fixture) => {
        const identity = expectJsonResult(
          await fixture.runJsonCli<{ id: string }>(['name', 'upgrade-held', '--save'])
        );
        const prefix = path.join(fixture.root, 'extension prefix');
        const install = (artifact: ArtifactFixture, json = true) =>
          fixture.runCli([
            'extension',
            'install',
            'remote',
            '--yes',
            '--archive',
            artifact.archive,
            '--manifest',
            artifact.manifest,
            '--prefix',
            prefix,
            ...(json ? ['--json'] : []),
          ]);
        const first = await createArtifact(fixture, '0.1.0-alpha.1', new Uint8Array(), 'remote');
        expect(expectJsonResult(await install(first))).not.toHaveProperty('restartHint');
        // RemoteOwner runs this exact genuine old payload and owns all of its
        // processes. Installation versions are synthetic archive coordinates;
        // neither archive impersonates the binary's compiled provider version.
        const oldRelease = realpathSync(path.join(prefix, 'lib/tmt-remote/current'));
        const expectedRemote = readFileSync(path.resolve('../rust/target/debug/tmt-remote'));
        // Object equality expands every byte into an entry. Native Buffer
        // equality checks the complete debug payload without that allocation.
        expect(readFileSync(path.join(oldRelease, 'tmt-remote')).equals(expectedRemote)).toBe(true);
        symlinkSync(
          path.join(prefix, 'bin/tmt-remote'),
          path.join(fixture.wrapperDir, 'tmt-remote')
        );
        const owner = new RemoteOwner(fixture);
        try {
          await owner.start();
          const { device, paired } = await owner.pair(undefined, { talk: true });
          await owner.stop();
          owner.seedGrant(paired.clientId, { mode: 'hold' });
          await owner.start();
          const beforeStatus = expectJsonResult(
            await fixture.runCli(['remote', 'status', '--json'])
          );
          expect(beforeStatus).toMatchObject({ running: true });
          const session = await owner.session(device, paired);
          const operationId = randomUUID();
          expect(
            await session.append(
              'dispatch.create',
              dispatchIntent(operationId, identity.id, 'Still awaiting approval'),
              operationId
            )
          ).toEqual({ state: 'held', operationId });
          const approval = await owner.approval(operationId);
          const before = requestAttempts(fixture);

          for (const [version, json] of [
            ['0.1.0-alpha.2', true],
            ['0.1.0-alpha.3', false],
          ] as const) {
            const artifact = await createArtifact(fixture, version, new Uint8Array([1]), 'remote');
            const upgraded = await install(artifact, json);
            expect(upgraded.code, upgraded.stdout + upgraded.stderr).toBe(0);
            if (json)
              expect(upgraded.json).toMatchObject({ changed: true, version, restartHint: restart });
            else expect(upgraded.stdout).toContain(restart);
            expect(upgraded.stdout.split('Remote was upgraded').length - 1).toBe(1);
            expect(expectJsonResult(await install(artifact))).not.toHaveProperty('restartHint');
            expect(expectJsonResult(await fixture.runCli(['remote', 'status', '--json']))).toEqual(
              beforeStatus
            );
            expect(await session.append('operation.show', { operationId })).toEqual({
              state: 'held',
              operationId,
            });
            expect(requestAttempts(fixture)).toEqual(before);
            expect(readFileSync(path.join(oldRelease, 'tmt-remote')).equals(expectedRemote)).toBe(
              true
            );
          }
          const ended = await approval.finish('confirm');
          expect(ended).toMatchObject({ event: 'ended', state: 'accepted', operationId });
          expect(
            requestAttempts(fixture).filter((row) => row.request_id === ended.requestId)
          ).toHaveLength(1);
          expect(await session.append('operation.show', { operationId })).toMatchObject({
            state: 'accepted',
            requestId: ended.requestId,
          });
          await owner.stop();
          const stopped = await createArtifact(
            fixture,
            '0.1.0-alpha.4',
            new Uint8Array([2]),
            'remote'
          );
          expect(expectJsonResult(await install(stopped))).not.toHaveProperty('restartHint');
          expect(expectJsonResult(await fixture.runCli(['remote', 'status', '--json']))).toEqual({
            running: false,
            lastPort: Number(new URL(String(beforeStatus.origin)).port),
          });
        } finally {
          await owner.dispose();
        }
      },
      { mode: 'input-log' }
    );
  }, 45_000);
});
