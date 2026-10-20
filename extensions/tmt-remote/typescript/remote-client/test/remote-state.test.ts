import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { remoteState, currentRemote } from '../src/remote-state.js';

test('list and header use one current-origin state and the shared success role', () => {
  assert.deepEqual(remoteState('Connected'), {
    text: '● Connected',
    word: 'connected',
    mark: '●',
    tone: 'working',
    action: '',
  });
  assert.equal(remoteState('Checking').word, 'connecting');
  assert.equal(remoteState('Access ended').action, 'Pair again');
  for (const state of ['Not accepted', 'Unconfirmed']) {
    assert.equal(remoteState(state).word, 'unreachable');
    assert.equal(remoteState(state).action, 'Retry');
  }
  assert.equal(remoteState('Different machine').word, 'different machine');
  assert.equal(remoteState('Different machine').action, 'Pair again');
  for (const state of ['Connected', 'Access ended', 'Checking']) {
    assert.deepEqual(remoteState(state, false), {
      text: 'not checked',
      word: 'not checked',
      mark: '',
      tone: 'muted',
      action: '',
    });
  }
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
