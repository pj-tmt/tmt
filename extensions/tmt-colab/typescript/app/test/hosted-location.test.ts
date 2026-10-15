import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { encodeBinary } from '@tmt/colab-client';
import { captureHostedLink, isHostedOrigin } from '../src/hosted-location.js';
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/authority-v1.json', import.meta.url), 'utf8'),
);
const hex = (value: string) =>
  Uint8Array.from(value.match(/../g) ?? [], (byte) => parseInt(byte, 16));
const fragment = () =>
  '#' +
  Object.entries({
    v: '1',
    space: v.space,
    page: v.page,
    link: v.linkId,
    rev: '1',
    st: encodeBinary(hex(v.statementHash)),
    seed: encodeBinary(hex(v.linkSeed)),
    root: encodeBinary(hex(v.public)),
    role: 'viewer',
  })
    .map(([key, value]) => `${key}=${value}`)
    .join('&');
it.each([
  ['https://demo-example.web.app', true],
  ['https://demo-example.firebaseapp.com', true],
  ['http://127.0.0.1:1234', false],
  ['https://door.example.test', false],
  ['https://demo-example.web.app.attacker.test', false],
  ['https://attackerweb.app', false],
])('only Firebase Hosting origins replace the ordinary preview (%s)', (origin, expected) =>
  expect(isHostedOrigin(origin)).toBe(expected),
);
it('erases the fragment synchronously before the Colab parser finishes, preserving only its typed result', async () => {
  let selected = fragment();
  const calls: unknown[] = [];
  const browser = {
    location: {
      get hash() {
        return selected;
      },
      pathname: '/read/page',
      search: '',
    },
    history: {
      state: null,
      replaceState(...args: unknown[]) {
        calls.push(args);
        selected = '';
      },
    },
  };
  const parsed = captureHostedLink(browser);
  expect(selected).toBe('');
  expect(calls).toEqual([[null, '', '/read/page']]);
  const target = await parsed;
  expect(target).toMatchObject({ space: v.space, page: v.page, link: v.linkId, role: 'viewer' });
  expect(target?.root).toEqual(hex(v.public));
  target?.seed.fill(0);
});
it('an empty entry returns no target rather than manufacturing a sample page', async () => {
  expect(
    await captureHostedLink({
      location: { hash: '', pathname: '/', search: '' },
      history: { state: null, replaceState() {} },
    }),
  ).toBeUndefined();
});
it('older and malformed fragments are removed and refuse without a compatibility parser', async () => {
  for (const hash of [fragment().replace(/&root=[^&]+&role=viewer$/, ''), '#bad=secret']) {
    let erased = false;
    await expect(
      captureHostedLink({
        location: { hash, pathname: '/', search: '' },
        history: {
          state: null,
          replaceState() {
            erased = true;
          },
        },
      }),
    ).rejects.toBeDefined();
    expect(erased).toBe(true);
  }
});
