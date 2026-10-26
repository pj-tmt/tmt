import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { deploymentCommands, serviceCommands } from '../src/service-commands.js';

test('guide commands preserve native defaults and keep preview separate from terminal consent', () => {
  assert.deepEqual(deploymentCommands('demo-guide', 'asia-east1'), {
    preview:
      "tmt remote deploy firestore --project='demo-guide' --region='asia-east1' --defaults --json",
    deploy: "tmt remote deploy firestore --project='demo-guide' --region='asia-east1' --defaults",
  });
  assert.equal(serviceCommands.nodeVersion, 'node --version');
  assert.equal(serviceCommands.firebaseVersion, 'firebase --version');
  assert.equal(serviceCommands.firebaseInstall, 'npm install -g firebase-tools@15.29.0');
  assert.equal(serviceCommands.firebaseLogin, 'firebase login');
  assert.equal(
    Object.values(serviceCommands).some((value) => value.includes('--authorize')),
    false,
  );
});

test('invalid or placeholder inputs cannot become a copied terminal command', () => {
  for (const project of [
    '',
    '<project-id>',
    'short',
    'Demo-guide',
    'demo;false',
    "demo'quote",
    'a'.repeat(31),
  ])
    assert.throws(() => deploymentCommands(project, 'asia-east1'));
  for (const region of [
    '',
    '<region>',
    'asia east1',
    'asia;false',
    '$(false)',
    'ASIA-EAST1',
    'a'.repeat(33),
  ])
    assert.throws(() => deploymentCommands('demo-guide', region));
});
