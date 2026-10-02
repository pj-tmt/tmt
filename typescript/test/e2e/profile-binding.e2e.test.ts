import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';

const inputLog = { mode: 'input-log' } as const;

describe('profile ownership across binding transitions', { concurrent: false }, () => {
  it('uses the verified temporary caller for metadata and rejects tampered caller writes', async () => {
    await withE2EFixture(async (fixture) => {
      const named = await fixture.runJsonCli<{ id: string; lifetime: string }>(['name', 'Alice']);
      expect(named).toMatchObject({
        code: 0,
        json: { id: expect.any(String), lifetime: 'temporary' },
      });
      const identityId = named.json!.id;

      expect(await fixture.runJsonCli(['identity', 'show'])).toMatchObject({
        code: 0,
        json: { identity: { id: identityId, name: 'Alice', lifetime: 'temporary' } },
      });

      expect(await fixture.runJsonCli(['identity', 'meta', 'set', 'project', 'tmt'])).toMatchObject(
        {
          code: 0,
          json: { identityId, key: 'project', value: 'tmt', changed: true },
        }
      );
      expect(await fixture.runJsonCli(['identity', 'meta', 'get', 'project'])).toMatchObject({
        code: 0,
        json: { identityId, key: 'project', value: 'tmt' },
      });
      expect(await fixture.runJsonCli(['identity', 'meta', 'list'])).toMatchObject({
        code: 0,
        json: { identityId, metadata: { project: 'tmt' } },
      });
      expect(durableState(fixture)).toMatchObject({
        identities: [expect.objectContaining({ id: identityId, lifetime: 'temporary' })],
        metadata: [{ identity_id: identityId, key: 'project', value: 'tmt' }],
      });

      const paneMetadata = JSON.parse(fixture.paneMetadata());
      paneMetadata.globalIdentity.bindingId = 'tampered-binding';
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmux-team.agent',
        JSON.stringify(paneMetadata),
      ]);
      expect(
        await fixture.runJsonCli(['identity', 'meta', 'set', 'project', 'other'])
      ).toMatchObject({
        code: 1,
        json: { error: { code: 'IDENTITY_REQUIRED' } },
      });
      const rejectedShow = await fixture.runJsonCli<{ error: { message: string } }>([
        'identity',
        'show',
      ]);
      expect(rejectedShow).toMatchObject({
        code: 1,
        json: { error: { code: 'IDENTITY_REQUIRED' } },
      });
      expect(rejectedShow.json?.error?.message).toContain('identity show <name>');
      expect(durableState(fixture)).toMatchObject({
        identities: [expect.objectContaining({ id: identityId, lifetime: 'temporary' })],
        metadata: [{ identity_id: identityId, key: 'project', value: 'tmt' }],
      });
    }, inputLog);
  });

  it('uses a verified implicit role and preserves saved profiles across unbind and rebind', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Alice', '-s'])).toMatchObject({ code: 0 });
      const initial = await fixture.runJsonCli(['role', 'set', 'saved role']);
      expect(initial).toMatchObject({
        code: 0,
        json: { identity: { name: 'Alice' }, role: { content: 'saved role' } },
      });
      expect(
        await fixture.runJsonCli(['preamble', 'set', 'Alice', 'saved preamble'])
      ).toMatchObject({ code: 0 });
      const metadata = fixture.paneMetadata();
      expect(await fixture.runJsonCli(['identity', 'create', 'Offline'])).toMatchObject({
        code: 0,
      });
      const offline = await fixture.runJsonCli(['identity', 'show', 'Offline'], {
        withoutTmux: true,
      });
      expect(offline).toMatchObject({
        code: 0,
        json: { identity: { name: 'Offline', lifetime: 'saved' } },
      });
      expect(await fixture.runJsonCli(['identity', 'show', 'Offline'])).toEqual(offline);
      expect(await fixture.runJsonCli(['identity', 'show'])).toMatchObject({
        code: 0,
        json: { identity: { name: 'Alice', lifetime: 'saved' } },
      });
      expect(
        await fixture.runJsonCli(['role', 'set', 'offline role', '--identity', 'Offline'])
      ).toMatchObject({ code: 0, json: { identity: { name: 'Offline' } } });
      expect(fixture.paneMetadata()).toBe(metadata);
      expect((await fixture.runJsonCli(['role', 'show'])).json).toEqual(initial.json);
      expect(await fixture.runJsonCli(['unbind'])).toMatchObject({ code: 0 });
      expect(await fixture.runJsonCli(['identity', 'show'])).toMatchObject({
        code: 1,
        json: { error: { code: 'IDENTITY_REQUIRED' } },
      });
      expect(await fixture.runJsonCli(['role', 'set', 'must not write'])).toMatchObject({
        code: 1,
        json: { error: { code: 'IDENTITY_REQUIRED' } },
      });
      expect(
        (await fixture.runJsonCli(['role', 'show', '--identity', 'Alice'], { withoutTmux: true }))
          .json
      ).toEqual(initial.json);
      const peer = await fixture.createMockPane('rebound');
      expect(await fixture.runJsonCli(['add', peer.pane, 'Alice'])).toMatchObject({ code: 0 });
      expect((await fixture.runJsonCli(['role', 'show', '--identity', 'Alice'])).json).toEqual(
        initial.json
      );
      expect(await fixture.runJsonCli(['preamble', 'show', 'Alice'])).toMatchObject({
        code: 0,
        json: { preamble: 'saved preamble' },
      });
      expect(await fixture.runJsonCli(['rm', 'Alice', '--force'])).toMatchObject({ code: 0 });
      expect(await fixture.runJsonCli(['preamble'], { withoutTmux: true })).toMatchObject({
        code: 0,
        json: { preambles: [] },
      });
      expect(durableState(fixture).profiles).toEqual([
        expect.objectContaining({ content: 'offline role' }),
      ]);
      expect(fixture.paneMetadata(peer.pane)).toBe('');
    }, inputLog);
  });

  it('does not mutate profiles when caller metadata no longer verifies', async () => {
    await withE2EFixture(async (fixture) => {
      expect(await fixture.runJsonCli(['name', 'Alice', '-s'])).toMatchObject({ code: 0 });
      const original = await fixture.runJsonCli(['role', 'set', 'preserved']);
      expect(original.code).toBe(0);
      const profiles = durableState(fixture).profiles;
      expect(profiles).toEqual([expect.objectContaining({ content: 'preserved' })]);
      const metadata = JSON.parse(fixture.paneMetadata());
      metadata.globalIdentity.bindingId = 'different-binding';
      fixture.tmux([
        'set-option',
        '-p',
        '-t',
        fixture.pane,
        '@tmux-team.agent',
        JSON.stringify(metadata),
      ]);
      const rejected = await fixture.runJsonCli(['role', 'clear']);
      expect(rejected).toMatchObject({ code: 1, json: { error: { code: 'IDENTITY_REQUIRED' } } });
      expect(
        (await fixture.runJsonCli(['role', 'show', '--identity', 'Alice'], { withoutTmux: true }))
          .json
      ).toEqual(original.json);
      expect(durableState(fixture).bindings).toEqual([]);
      expect(durableState(fixture).profiles).toEqual(profiles);
      expect(JSON.parse(fixture.paneMetadata())).toEqual(metadata);
    }, inputLog);
  });

  it('hides retired temporary profiles without leaking them to a reused name', async () => {
    await withE2EFixture(async (fixture) => {
      const old = await fixture.runJsonCli<{ id: string }>(['name', 'Temporary']);
      expect(old.code).toBe(0);
      expect(old.json?.id).toEqual(expect.any(String));
      expect(await fixture.runJsonCli(['role', 'set', 'old role'])).toMatchObject({ code: 0 });
      expect(
        await fixture.runJsonCli(['preamble', 'set', 'Temporary', 'old preamble'])
      ).toMatchObject({ code: 0 });
      expect(await fixture.runJsonCli(['unbind'])).toMatchObject({
        code: 0,
        json: { retired: true },
      });
      expect(await fixture.runJsonCli(['preamble'], { withoutTmux: true })).toMatchObject({
        code: 0,
        json: { preambles: [] },
      });
      expect(
        await fixture.runJsonCli(['role', 'show', '--identity', 'Temporary'], {
          withoutTmux: true,
        })
      ).toMatchObject({ code: 3, json: { error: { code: 'NAME_NOT_FOUND' } } });
      const fresh = await fixture.runJsonCli<{ id: string }>(['name', 'Temporary']);
      expect(fresh.code).toBe(0);
      expect(fresh.json?.id).not.toBe(old.json?.id);
      expect(await fixture.runJsonCli(['role', 'show'])).toMatchObject({
        code: 0,
        json: { role: null },
      });
      expect(await fixture.runJsonCli(['preamble', 'show', 'Temporary'])).toMatchObject({
        code: 0,
        json: { preamble: null },
      });
      expect(durableState(fixture).profiles).toEqual([
        expect.objectContaining({ identity_id: old.json?.id, content: 'old role' }),
      ]);
    }, inputLog);
  });
});
