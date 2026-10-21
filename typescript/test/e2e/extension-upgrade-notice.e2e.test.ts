import { randomBytes, randomUUID } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, realpathSync, symlinkSync } from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { syntheticAlphaVersion } from '../../scripts/release-versions.mjs';
import { createArtifact, type ArtifactFixture } from '../support/native-artifact.js';
import { runCli, withSandbox } from '../support/cli-process.js';
import { install as installCore } from '../support/native-installation.js';
import { requestEnvelope, text, verifyResponse } from '../support/remote-device.js';
import { dispatchIntent, post, RemoteOwner, RemoteSession } from '../support/remote-owner.js';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';

const restart =
  'Remote was upgraded while its door is running. Finish active pairing and held approvals, then restart: tmt remote stop && tmt remote serve.';

describe('post-upgrade Remote restart', () => {
  it('restarts at the same endpoint with intact grants and keeps calling Core after its old release is pruned', async () => {
    await withSandbox(async (sandbox) => {
      const version = await runCli(sandbox, ['--version']);
      expect(version.status, version.stderr).toBe(0);
      // Synthetic alpha metadata admits this real debug payload to the install
      // channel. Use the historical no-companion archive shape: Docker does not
      // build the independently packaged Herdr driver.
      const core = await createArtifact(
        { ...sandbox, cli: { ...sandbox.cli, companions: null } },
        syntheticAlphaVersion(version.stdout.trim())
      );
      const corePrefix = path.join(sandbox.root, 'Core prefix');
      const installed = await installCore(sandbox, core, corePrefix);
      const originalCore = realpathSync(installed.executable);
      await withE2EFixture(
        async (fixture) => {
          const prefix = path.join(fixture.root, 'background extension prefix');
          const install = async (artifact: ArtifactFixture) =>
            expectJsonResult(
              await fixture.runCli([
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
                '--json',
              ])
            );
          const first = await createArtifact(fixture, '0.1.0-alpha.1', new Uint8Array(), 'remote');
          expect(await install(first)).not.toHaveProperty('restartHint');
          symlinkSync(
            path.join(prefix, 'bin/tmt-remote'),
            path.join(fixture.wrapperDir, 'tmt-remote')
          );
          const identity = expectJsonResult(
            await fixture.runJsonCli<{ id: string }>(['name', 'core-pruning-control', '--save'])
          );
          const owner = new RemoteOwner(fixture);
          let background = false;
          try {
            await owner.start();
            const { device, paired } = await owner.pair();
            await owner.stop();
            const grantBytes = () => {
              const db = new Database(path.join(fixture.globalDir, 'remote/remote.db'), {
                readonly: true,
                fileMustExist: true,
              });
              try {
                return Buffer.from(
                  JSON.stringify(db.prepare('SELECT * FROM grants ORDER BY client_id').all())
                );
              } finally {
                db.close();
              }
            };
            const grants = grantBytes();
            const oldRelease = realpathSync(path.join(prefix, 'lib/tmt-remote/current'));
            background = true; // Cleanup owns even an uncertain handoff.
            const ready = expectJsonResult(
              await fixture.runCli(['remote', 'serve', '--background', '--json'])
            );
            const beforeStatus = expectJsonResult(
              await fixture.runCli(['remote', 'status', '--json'])
            );
            const second = await createArtifact(
              fixture,
              '0.1.0-alpha.2',
              new Uint8Array([1]),
              'remote'
            );
            const upgraded = await install(second);
            expect(upgraded).toMatchObject({
              changed: true,
              restartHint: 'Remote upgraded and restarted in the background.',
            });
            expect(expectJsonResult(await fixture.runCli(['remote', 'status', '--json']))).toEqual(
              beforeStatus
            );
            const current = realpathSync(path.join(prefix, 'lib/tmt-remote/current'));
            expect(current).not.toBe(oldRelease);
            const oldBytes = readFileSync(path.join(oldRelease, 'tmt-remote'));
            expect(
              readFileSync(path.join(current, 'tmt-remote')).equals(
                Buffer.concat([oldBytes, Buffer.from([1])])
              )
            ).toBe(true);
            const origin = String(beforeStatus.origin);
            const response = await fetch(`${origin}/sdk/mount`, {
              method: 'POST',
              headers: { Origin: origin, 'content-type': 'application/json' },
              body: JSON.stringify({ path: '/' }),
            });
            expect(response.status).toBe(200);
            const admission = (await response.json()) as Record<string, unknown>;
            expect(admission.address).toBe(ready.address);
            expect(admission.machineId).toBe(ready.machineId);
            expect(admission.windowId).not.toBe(ready.windowId);
            expect(admission.windowId).toMatch(
              /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/
            );
            expect(grantBytes().equals(grants)).toBe(true);
            // This is the real restarted door, not RemoteOwner's foreground process.
            // Open against its newly admitted window and keep that same session alive.
            const backgroundDoor: Pick<RemoteOwner, 'post'> = {
              post: (route, input) => post(`${text(admission.address)}/${route}`, input),
            };
            const windowId = text(admission.windowId);
            const opening = requestEnvelope(
              device,
              paired,
              windowId,
              'new',
              '0',
              'session.open',
              { clientNonce: randomBytes(16).toString('hex') },
              { kind: 'control' }
            );
            const opened = await backgroundDoor.post('append', opening);
            expect(opened.status).toBe(200);
            const openedSession = verifyResponse(opened.body, opening, paired);
            const session = new RemoteSession(
              backgroundDoor,
              device,
              paired,
              text(openedSession.payload.sessionId),
              windowId,
              openedSession.envelope.sequence
            );
            const expectedIdentity = expect.objectContaining({
              id: identity.id,
              name: 'core-pruning-control',
            });
            expect((await session.append('agents.list', {})).identities).toContainEqual(
              expectedIdentity
            );
            // Metadata activations of this verified archive allocate generations just
            // like an upgrade. The third activation prunes the door's original Core.
            await installCore(sandbox, core, corePrefix, ['--pin']);
            await installCore(sandbox, core, corePrefix, ['--unpin']);
            expect(existsSync(path.dirname(originalCore))).toBe(false);
            expect(realpathSync(installed.executable)).not.toBe(originalCore);
            expect((await session.append('agents.list', {})).identities).toContainEqual(
              expectedIdentity
            );
            // Reuse the exact artifact: equal-version installs still verify integrity.
            expect(await install(second)).not.toHaveProperty('restartHint');
          } finally {
            try {
              if (background)
                expect(
                  expectJsonResult(await fixture.runCli(['remote', 'stop', '--json']))
                ).toEqual({
                  stopped: true,
                });
            } finally {
              await owner.dispose();
            }
          }
        },
        {
          mode: 'input-log',
          executableEnv: {
            TMT_TEST_CLI: JSON.stringify({ executable: installed.executable, args: [] }),
          },
        }
      );
    });
  }, 90_000);

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

          let previousRelease = oldRelease;
          let previousRemote = expectedRemote;
          const upgradedRemote = Buffer.concat([expectedRemote, Buffer.from([1])]);
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
            const activeRelease = realpathSync(path.join(prefix, 'lib/tmt-remote/current'));
            expect(readdirSync(path.join(prefix, 'lib/tmt-remote/releases')).sort()).toEqual(
              [path.basename(previousRelease), path.basename(activeRelease)].sort()
            );
            expect(
              readFileSync(path.join(previousRelease, 'tmt-remote')).equals(previousRemote)
            ).toBe(true);
            expect(
              readFileSync(path.join(activeRelease, 'tmt-remote')).equals(upgradedRemote)
            ).toBe(true);
            expect(existsSync(oldRelease)).toBe(previousRelease === oldRelease);
            previousRelease = activeRelease;
            previousRemote = upgradedRemote;
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
            signIn: { state: 'none', expiresAtMs: null },
          });
        } finally {
          await owner.dispose();
        }
      },
      { mode: 'input-log' }
    );
  }, 45_000);
});
