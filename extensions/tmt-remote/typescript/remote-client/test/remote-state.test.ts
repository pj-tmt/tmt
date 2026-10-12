import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { remoteState } from '../src/remote-state.js';

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
