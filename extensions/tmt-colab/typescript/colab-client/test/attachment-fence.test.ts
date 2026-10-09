import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { attachment as a } from '../src/index.js';

const corpus = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/attachment-fence-v1.json', import.meta.url),
    'utf8',
  ),
);
const input = (c: Record<string, string>) => ({
  space: c.space!,
  page: c.page!,
  epoch: c.epoch!,
  membershipRevision: c.revision!,
  membershipHash: Uint8Array.from(Buffer.from(c.headHash!, 'hex')),
  author: c.author!,
});

it('computes the same message fence as the independent oracle and Rust', async () => {
  expect(corpus.cases.length).toBeGreaterThanOrEqual(7);
  for (const c of corpus.cases) expect(await a.messageFence(input(c))).toBe(c.fence);
});

it('refuses malformed inputs instead of hashing them', async () => {
  const base = corpus.cases[0];
  for (const [name, value] of [
    ['space', 'short'],
    ['page', 'not-an-id'],
    ['epoch', '0'],
    ['revision', '01'],
    ['author', 'x'],
  ] as const)
    await expect(a.messageFence(input({ ...base, [name]: value }))).rejects.toThrow();
  await expect(
    a.messageFence({ ...input(base), membershipHash: new Uint8Array(31) }),
  ).rejects.toThrow();
});
