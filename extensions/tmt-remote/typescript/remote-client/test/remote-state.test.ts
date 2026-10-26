import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { remoteState, currentRemote, remoteRole, managementRole } from '../src/remote-state.js';

test('connection words and marks require current explicit observations', () => {
  assert.equal(remoteState('Connected').mark, '●');
  assert.equal(remoteState('Connected').text, 'Connected');
  assert.equal(remoteState('Checking').mark, '◌');
  assert.equal(remoteState('Offline').word, 'offline');
  assert.equal(remoteState('Access ended').action, 'Pair again');
  assert.equal(remoteState('Different machine').word, 'different machine');
  assert.equal(remoteState('Unconfirmed').action, 'Retry');
  assert.equal(remoteState('Not accepted').text, 'Not accepted');
  for (const value of ['something not reported', 'constructor', '__proto__'])
    assert.equal(remoteState(value).word, 'unknown');
  assert.equal(remoteState('something not reported').action, '');
  for (const state of ['Connected', 'Access ended', 'Checking', 'Offline']) {
    assert.deepEqual(remoteState(state, false), {
      text: 'Not checked',
      word: 'not checked',
      mark: '◌',
      tone: 'muted',
      action: '',
    });
  }
});

test('roles use the approved display labels without guessing from a capability or pairing', () => {
  assert.equal(remoteRole('owner'), 'Admin');
  assert.equal(remoteRole('user'), 'Member');
  for (const value of [undefined, null, true, 'browser', 'paired', 'admin', { talk: true }])
    assert.equal(remoteRole(value), 'Role unknown');
});

test('current remote uses the same door root for prefix and root pages', () => {
  const root = { kind: 'local', url: 'http://localhost:9001/' } as const;
  const prefix = { kind: 'local', url: 'http://localhost:9001/r/aaaaaaaaaaaaaaaa/' } as const;
  const prefixPage = `${prefix.url}settings`;
  assert.equal(currentRemote(prefix, undefined, prefixPage), true);
  assert.equal(currentRemote(root, undefined, prefixPage), false);
  assert.equal(currentRemote(prefix, prefix.url, prefixPage), true);
  assert.equal(currentRemote(root, prefix.url, prefixPage), false);
  assert.equal(currentRemote(root, undefined, `${root.url}settings`), true);
  assert.equal(currentRemote(prefix, undefined, `${root.url}settings`), false);
  assert.equal(currentRemote({ ...root, kind: 'firestore' }, root.url, root.url), false);
});

test('moved Remote pages retain the saved door-root identity without marking another origin', () => {
  const root = { kind: 'local', url: 'http://localhost:9001/' } as const;
  const foreign = { kind: 'local', url: 'http://localhost:9002/' } as const;
  for (const path of ['remote/', 'remote/settings']) {
    const page = `${root.url}${path}`;
    assert.equal(currentRemote(root, undefined, page), true);
    assert.equal(currentRemote(root, `${root.url}r/aaaaaaaaaaaaaaaa`, page), true);
    assert.equal(currentRemote(foreign, undefined, page), false);
    assert.equal(currentRemote({ ...root, kind: 'firestore' }, root.url, page), false);
  }
});

test('management observations display Admin, Member or unknown without widening authority', () => {
  assert.equal(managementRole('live', true), 'owner');
  assert.equal(managementRole('live', false), 'user');
  assert.equal(managementRole('live', undefined), undefined);
  for (const access of ['checking', 'lost', 'unconfirmed']) {
    assert.equal(managementRole(access, true), undefined);
    assert.equal(managementRole(access, false), undefined);
  }
});

test('Remote routes separate the global list and current-door tabs without obsolete aliases', async () => {
  const { REMOTE_ITEMS, REMOTE_PATHS, remoteItem, remoteTabAccess } =
    await import('../src/remote-routes.js');
  assert.deepEqual(
    REMOTE_ITEMS.map((item) => item.path),
    [
      '/remote/',
      '/remote/create',
      '/remote/overview',
      '/remote/members',
      '/remote/invites',
      '/remote/my-devices',
      '/remote/settings',
      '/remote/service',
    ],
  );
  assert.equal(remoteItem('/remote')?.id, 'remotes');
  for (const old of ['/remote/machine', '/remote/devices', '/remote/connect', '/remote/unknown'])
    assert.equal(remoteItem(old), undefined);
  assert.equal(new Set(REMOTE_PATHS).size, REMOTE_PATHS.length);
  assert.equal(REMOTE_PATHS.at(-1), '/remote/');
  for (const item of REMOTE_ITEMS) {
    assert.equal(remoteTabAccess(item, 'owner').locked, false);
    if (item.adminOnly) {
      assert.equal(remoteTabAccess(item, 'user').locked, true);
      assert.match(remoteTabAccess(item, 'user').reason, /Ask the person who manages/);
      assert.ok(remoteTabAccess(item, 'user').request);
      assert.equal(remoteTabAccess(item, undefined).request, '');
      assert.match(remoteTabAccess(item, undefined).reason, /not confirmed/);
    } else assert.equal(remoteTabAccess(item, 'user').locked, false);
  }
});

test('one pairing guidance owner preserves uncertainty and verified-refusal boundaries', async () => {
  const { pairingGuidance } = await import('../src/remote-state.js');
  assert.equal(pairingGuidance('missing', false, false).pair, true);
  assert.equal(pairingGuidance('different', false, false).pair, true);
  assert.deepEqual(pairingGuidance('refused', true, false), {
    pair: true,
    members: true,
    status: false,
  });
  for (const state of ['unreadable', 'checking', 'connected', 'refused'])
    assert.equal(pairingGuidance(state, false, false).pair, false);
  assert.deepEqual(pairingGuidance('unconfirmed', false, false), {
    pair: false,
    members: false,
    status: true,
  });
  assert.equal(pairingGuidance('connected', false, true).pair, true);
});
