import { expect, it } from 'vite-plus/test';
import { moduleLoad } from '../src/module-entry.js';

const HERE = 'http://127.0.0.1:4000/colab/';
const MOUNT = '/r/abcdefgh/x/colab/';
const ENTRY = `http://127.0.0.1:4000${MOUNT}assets/colab-module.js`;

it('loads the default entry, or a named file, under the module mount', () => {
  expect(moduleLoad(MOUNT, undefined, HERE)).toEqual({ link: MOUNT, entry: ENTRY });
  expect(moduleLoad(MOUNT, 'assets/other.js', HERE).entry).toBe(
    `http://127.0.0.1:4000${MOUNT}assets/other.js`,
  );
  expect(moduleLoad(MOUNT, `${MOUNT}assets/colab-module.js`, HERE).entry).toBe(ENTRY);
  // A mount written without its trailing slash is the same mount.
  expect(moduleLoad('/r/abcdefgh/x/colab', undefined, HERE)).toEqual({ link: MOUNT, entry: ENTRY });
  expect(moduleLoad(`http://127.0.0.1:4000${MOUNT}`, undefined, HERE).entry).toBe(ENTRY);
});

it('refuses an entry outside the mount and keeps the module link', () => {
  for (const entry of [
    'https://evil.example/colab.js',
    '//evil.example/colab.js',
    '/evil/colab.js',
    '../../colab.js',
    '../colab/assets/x.js',
    './assets/x.js',
    'assets/../x.js',
    '/r/abcdefgh/x/other/colab.js',
    '/r/abcdefgh/x/colab-evil/colab.js',
    '/r/zzzzzzzz/x/colab/colab.js',
    'assets//x.js',
    'assets/colab.css',
    'assets/colab-module',
    '',
    '.',
  ])
    expect(moduleLoad(MOUNT, entry, HERE), entry).toEqual({ link: MOUNT });
});

it('refuses escapes, backslashes, queries, fragments and control characters', () => {
  for (const entry of [
    'assets%2Fx.js',
    'assets%2fx.js',
    '%2e%2e/x.js',
    'assets/%2E%2E/x.js',
    'assets\\x.js',
    'assets/x.js?v=1',
    'assets/x.js#a',
    'assets/x.js\n',
    'assets/x\u0000.js',
    `${MOUNT}`,
  ])
    expect(moduleLoad(MOUNT, entry, HERE).entry, JSON.stringify(entry)).toBeUndefined();
});

it('offers no link or entry for a mount that is not a module mount on this origin', () => {
  for (const rail of [
    'https://evil.example/r/abcdefgh/x/colab/',
    '/evil/',
    '/r/abcdefgh/x/',
    '/r/abcdefgh/x/colab/extra/',
    '/r/abcdefgh/x/colab/?q=1',
    '/r/abcdefgh/x/colab/#a',
    '/r/ABC/x/colab/',
    '',
  ])
    expect(moduleLoad(rail, undefined, HERE), rail).toEqual({});
  expect(moduleLoad(MOUNT, undefined, 'not a url')).toEqual({});
});
