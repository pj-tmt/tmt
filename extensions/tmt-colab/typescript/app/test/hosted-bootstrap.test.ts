import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import { digest, text } from '@tmt/colab-client';
import { verifyHostedDeclaration } from '../../../../tmt-remote/typescript/remote-client/src/firebase-declaration.js';
import type { HostedAuth } from '../../../../tmt-remote/typescript/remote-client/src/firebase-auth.js';
import * as math from '../../../../tmt-remote/typescript/remote-client/src/budget.js';
import { bootstrapHosted } from '../src/hosted-bootstrap.js';
import { responseBytes, jsonResponse } from '../src/registration.js';

const vector = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/deploy-declaration-v1.json', import.meta.url),
    'utf8',
  ),
);
const declaration = JSON.parse(vector.declaration);
const bytes = ` \n${vector.declaration}\n`;
const hash = async (raw: Uint8Array) =>
  [...(await digest(raw))].map((value) => value.toString(16).padStart(2, '0')).join('');
const call = () => ({ signal: new AbortController().signal, deadlineAtMs: Date.now() + 30_000 });
function fixture(response = new Response(bytes)) {
  const requests: unknown[] = [],
    declarations: unknown[] = [];
  const auth = { signInAnonymous: vi.fn(), close: vi.fn(async () => {}) } as unknown as HostedAuth;
  const fetcher = vi.fn(async (...args: unknown[]) => {
    requests.push(args);
    return response;
  }) as unknown as typeof fetch;
  const link = 'https://demo-example.web.app/pair#canonical';
  const originLink = vi.fn(async () => link);
  const connect = vi.fn(
    async (selected: string, declaration: { bytes: string; digest: string }) => {
      expect(selected).toBe(link);
      await verifyHostedDeclaration(declaration);
      declarations.push({ ...declaration });
      return auth;
    },
  );
  const options = {
    fetch: fetcher,
    load: async () => ({ firebaseRemote: { originLink, connect }, budget: math }),
  };
  return { auth, options, requests, declarations, originLink, connect };
}

it('binds the one exact served declaration and derives every run limit from those same bytes', async () => {
  const f = fixture(),
    boot = await bootstrapHosted(call(), f.options);
  expect(f.requests).toHaveLength(1);
  expect((f.requests[0] as unknown[])[0]).toBe('/tmt/declarations/colab.json');
  expect(f.originLink).toHaveBeenCalledOnce();
  expect(f.connect).toHaveBeenCalledOnce();
  expect(f.declarations).toEqual([{ bytes, digest: await hash(text(bytes)) }]);
  expect(boot.declaration).toEqual(f.declarations[0]);
  for (const resource of declaration.resources)
    expect(boot.limits.get(resource.name)).toEqual({
      documents: resource.limits.maxEntries,
      bytes: resource.limits.maxNamespaceBytes,
    });
  expect(boot.auth).toBe(f.auth);
  expect(f.auth.signInAnonymous).not.toHaveBeenCalled();
  await boot.auth.close();
});

it.each([
  { response: () => new Response('not found', { status: 404 }) },
  { response: () => new Response(new Uint8Array(65537)) },
  { response: () => new Response(Uint8Array.of(255)) },
  { response: () => new Response('{}') },
])(
  'refuses a missing, oversized, non-UTF8 or invalid declaration without sign-in',
  async ({ response }) => {
    const f = fixture(response());
    await expect(bootstrapHosted(call(), f.options)).rejects.toBeDefined();
    expect(f.auth.signInAnonymous).not.toHaveBeenCalled();
  },
);

it('never creates Auth or reads a declaration after caller cancellation', async () => {
  const f = fixture();
  await expect(
    bootstrapHosted({ ...call(), signal: AbortSignal.abort() }, f.options),
  ).rejects.toMatchObject({ state: 'cancelled' });
  expect(f.requests).toEqual([]);
  expect(f.connect).not.toHaveBeenCalled();
});

it('closes an initialized Auth owner if cancellation arrives during connect', async () => {
  const f = fixture(),
    controller = new AbortController();
  const connect = f.options.load;
  f.options.load = async () => {
    const sdk = await connect();
    return {
      ...sdk,
      firebaseRemote: {
        ...sdk.firebaseRemote,
        connect: vi.fn(async (...args: Parameters<typeof sdk.firebaseRemote.connect>) => {
          const auth = await sdk.firebaseRemote.connect(...args);
          controller.abort();
          return auth;
        }),
      },
    };
  };
  await expect(
    bootstrapHosted({ ...call(), signal: controller.signal }, f.options),
  ).rejects.toMatchObject({ state: 'cancelled' });
  expect(f.auth.close).toHaveBeenCalledOnce();
});

it('the common bounded response reader preserves exact bytes and JSON admission behavior', async () => {
  expect(await responseBytes(new Response(bytes), 65536)).toEqual(text(bytes));
  expect(await jsonResponse(new Response('{"ok":true}'), 20)).toEqual({ ok: true });
  await expect(responseBytes(new Response('too long'), 2)).rejects.toThrow('exceeds limit');
  await expect(
    jsonResponse(new Response('{"code":"DENIED"}', { status: 403 }), 64),
  ).rejects.toMatchObject({ code: 'DENIED', status: 403 });
});
