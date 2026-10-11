import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { test } from 'vite-plus/test';
import pin from '../firebase-build-pin.json' with { type: 'json' };
import pkg from '../package.json' with { type: 'json' };
test('generated hosted bytes match reviewed pins, licenses and Colab caps without Node dependencies', async () => {
  assert.equal(pin.version, 1);
  assert.equal(pkg.devDependencies.firebase, '13.0.0');
  const installed = JSON.parse(
    await readFile(new URL('../node_modules/firebase/package.json', import.meta.url), 'utf8'),
  );
  assert.equal(installed.version, pkg.devDependencies.firebase);
  assert.deepEqual(
    pin.files.map((f) => f.path),
    ['dist/remote-firebase-v1.js', 'dist/THIRD-PARTY-NOTICES.txt'],
  );
  let total = 0;
  for (const file of pin.files) {
    const bytes = await readFile(new URL(`../${file.path}`, import.meta.url));
    assert.equal(file.firebaseVersion, pkg.devDependencies.firebase);
    assert.equal(bytes.length, file.length);
    assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256);
    assert.ok(bytes.length < 2 * 1024 * 1024);
    total += bytes.length;
    if (file.path.endsWith('.js')) {
      assert.doesNotMatch(bytes.toString(), /node:|grpc-js|undici|sourceMappingURL/);
    } else {
      assert.match(bytes.toString(), /Apache License/);
      assert.match(
        bytes.toString(),
        /TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION/,
      );
      assert.match(bytes.toString(), /ISC License/);
    }
  }
  total += (
    await readFile(new URL('../../../rust/tmt-remote/assets/remote-v1.js', import.meta.url))
  ).length;
  assert.ok(total < 8 * 1024 * 1024);
});
