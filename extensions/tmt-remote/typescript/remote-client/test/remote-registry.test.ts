import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import {
  parseCatalog,
  parseHandoff,
  buildHandoff,
  planImport,
  mergeImport,
  reference,
} from '../src/remote-registry.js';

const a = {
  name: 'studio',
  kind: 'local',
  url: 'http://127.0.0.1:9001/r/aaaaaaaaaaaaaaaa/',
  project: null,
} as const;
const b = {
  name: 'desk',
  kind: 'local',
  url: 'http://127.0.0.1:9002/r/bbbbbbbbbbbbbbbb/',
  project: null,
} as const;
const catalog = { version: 1, entries: [a, b] };

test('registry reuses closed public references; it rejects private fields and partial packets', async () => {
  assert.deepEqual(await parseCatalog(catalog), catalog);
  for (const entry of [
    { ...a, name: 'local' },
    { ...a, name: 'Studio' },
    { ...a, name: 'a'.repeat(64) },
    { ...a, project: 'project' },
    { ...a, url: 'javascript:alert(1)' },
    { ...a, url: 'http://user:pass@localhost/' },
    { ...a, url: `${a.url}?key=secret` },
    { ...a, url: `${a.url}#private` },
    { ...a, handle: 'private' },
  ])
    await assert.rejects(parseCatalog({ version: 1, entries: [b, entry] }));
  await assert.rejects(parseCatalog({ ...catalog, device: 'secret' }));
  await assert.rejects(parseCatalog({ version: 1, entries: [a, { ...b, name: a.name }] }));
  await assert.rejects(parseCatalog({ version: 1, entries: [a, { ...a, name: 'other' }] }));
  await assert.rejects(
    parseCatalog({
      version: 1,
      entries: Array.from({ length: 17 }, (_, i) => ({
        ...a,
        name: `r${i}`,
        url: `http://localhost:${9000 + i}/`,
      })),
    }),
    /at most 16 remotes/,
  );
});

test('handoff is public-only, bounded and targeted', async () => {
  const url = await buildHandoff(b, catalog);
  const packet = await parseHandoff(new URL(url).hash);
  assert.deepEqual(packet.catalog, catalog);
  assert.equal(new URL(url).origin, new URL(b.url).origin);
  await assert.rejects(parseHandoff(new URL(url).hash + '&tmt-source=host'));
  await assert.rejects(parseHandoff(`#tmt-remotes=${'x'.repeat(8193)}`));
  await assert.rejects(buildHandoff({ ...b, url: 'http://attacker/next?return=secret' }, catalog));
});

test('navigation hints contain only Colab space/page IDs, never share seeds or paths', async () => {
  const hint = { spaceId: 'a'.repeat(32), pageId: '00000000-0000-4000-8000-000000000001' };
  const url = await buildHandoff(b, catalog, hint);
  assert.deepEqual((await parseHandoff(new URL(url).hash)).hint, hint);
  for (const invalid of [
    { ...hint, seed: 'secret' },
    { ...hint, pageId: '/p/secret' },
    { ...hint, spaceId: 'wrong' },
  ])
    await assert.rejects(buildHandoff(b, catalog, invalid));
});

test('import requires confirmation; collisions and duplicates never partially overwrite', async () => {
  const current = { version: 1, entries: [a] };
  const incoming = {
    version: 1,
    entries: [
      { ...b, name: 'studio' },
      { ...a, name: 'copy' },
    ],
  };
  const plan = await planImport(incoming, current);
  assert.equal(plan.rows[0]!.status, 'collision');
  assert.equal(plan.rows[0]!.suggestedName, 'studio-2');
  assert.equal(plan.rows[1]!.status, 'duplicate');
  assert.equal(plan.rows[1]!.existingName, 'studio');
  assert.deepEqual(current, { version: 1, entries: [a] });
  await assert.rejects(
    mergeImport(plan, ['studio', null]),
    /You already have a remote named studio/,
  );
  assert.deepEqual(await mergeImport(plan, ['studio-2', null]), {
    version: 1,
    entries: [a, { ...b, name: 'studio-2' }],
  });
  assert.deepEqual(await mergeImport(plan, [null, null]), current);
});

test('door addresses reject pairing fragments', async () => {
  const result = await reference('studio', a.url);
  assert.deepEqual(result, a);
  await assert.rejects(reference('studio', `${a.url}pair#one-time-secret`));
});

test('native catalog fixture and public Firebase descriptor share their single owners', async () => {
  const { readFileSync } = await import('node:fs');
  const fixture = JSON.parse(
    readFileSync(
      new URL('../../../rust/tmt-remote/tests/fixtures/browser-remotes.json', import.meta.url),
      'utf8',
    ),
  );
  assert.deepEqual(await parseCatalog(fixture.catalog), fixture.catalog);
  for (const alias of fixture.validNames) assert.equal((await reference(alias, a.url)).name, alias);
  for (const alias of fixture.invalidNames) await assert.rejects(reference(alias, a.url));
  const { base64url } = await import('../src/canonical-bytes.js');
  const descriptor = JSON.parse(
    readFileSync(
      new URL(
        '../../../rust/tmt-remote/tests/fixtures/remote_link/descriptor-v1.json',
        import.meta.url,
      ),
      'utf8',
    ),
  );
  const canonical = (value: unknown): string => {
    if (!value || typeof value !== 'object') return JSON.stringify(value);
    const r = value as Record<string, unknown>;
    return `{${Object.keys(r)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(r[key])}`)
      .join(',')}}`;
  };
  const link = `https://demo-remote-1.web.app/pair#${base64url(new TextEncoder().encode(canonical(descriptor)))}`;
  const hosted = await reference('tenant', link);
  assert.equal(hosted.project, 'demo-remote-1');
  assert.equal(hosted.kind, 'firestore');
  assert.equal(await buildHandoff(hosted, catalog), link);
  await assert.rejects(
    parseCatalog({ version: 1, entries: [{ ...hosted, project: 'another-project' }] }),
  );
  await assert.rejects(reference('tenant', link.slice(0, -1) + 'A'));
});

test('entire encoded handoff is bounded and duplicate members cannot collapse into valid data', async () => {
  const { base64url } = await import('../src/canonical-bytes.js');
  const packet = await buildHandoff(b, catalog);
  await assert.rejects(parseHandoff(new URL(packet).hash + '&tmt-remotes=duplicate'));
  await assert.rejects(parseHandoff(new URL(packet).hash + '&secret=CANARY'));
  const hint =
    '{"spaceId":"' +
    'a'.repeat(32) +
    '","pageId":"00000000-0000-4000-8000-000000000001","spaceId":"' +
    'a'.repeat(32) +
    '"}';
  await assert.rejects(
    parseHandoff(new URL(packet).hash + '&tmt-page=' + base64url(new TextEncoder().encode(hint))),
  );
  const large = {
    version: 1,
    entries: Array.from({ length: 4 }, (_, i) => ({
      ...a,
      name: `r${i}`,
      url: `http://localhost/${'a'.repeat(1600)}${i}/`,
    })),
  };
  // Valid catalog JSON fits storage, but its encoded fragment does not fit navigation.
  assert.ok(new TextEncoder().encode(JSON.stringify(large)).length < 8192);
  assert.equal((await parseCatalog(large)).entries.length, 4);
  await assert.rejects(buildHandoff(b, large), /too large to open together/);
  const under = {
    ...large,
    entries: large.entries.map((entry) => ({
      ...entry,
      url: entry.url.replace('a'.repeat(1600), 'a'.repeat(1455)),
    })),
  };
  const url = await buildHandoff(b, under);
  const bytes = new TextEncoder().encode(new URL(url).hash).length;
  assert.ok(bytes <= 8192 && bytes > 8000);
  assert.deepEqual((await parseHandoff(new URL(url).hash)).catalog, under);
  const tooLarge = '#tmt-remotes=' + base64url(new TextEncoder().encode(JSON.stringify(large)));
  await assert.rejects(parseHandoff(tooLarge));
});
