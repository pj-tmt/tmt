import { chromium, firefox, webkit } from 'playwright';
import ts from 'typescript';
import { spawnSync } from 'node:child_process';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { ENGINES, validateReport } from './gate.mjs';
const root = new URL('../', import.meta.url),
  vectors = new URL('../../contracts/vectors/', root);
const corpus = (await readFile(new URL('ed25519-829.jsonl', vectors), 'utf8'))
  .trim()
  .split('\n')
  .map(JSON.parse);
const fixture = JSON.parse(await readFile(new URL('model-v1.json', vectors), 'utf8'));
const authority = JSON.parse(await readFile(new URL('authority-v1.json', vectors), 'utf8'));
const ownerCases = JSON.parse(
  await readFile(new URL('owner-member-v1.json', vectors), 'utf8'),
).cases;
const fixtureEnvelope = {
  header: Buffer.from(fixture.header, 'hex').toString('base64url'),
  nonce: Buffer.alloc(12).toString('base64url'),
  ciphertext: Buffer.from(fixture.ciphertext, 'hex').toString('base64url'),
  signature: Buffer.from(fixture.signature, 'hex').toString('base64url'),
};
function native(input, example = 'browser_conformance') {
  const result = spawnSync(
    'cargo',
    [
      process.env.COLAB_RUST_TOOLCHAIN ?? '+1.97.0',
      'run',
      '--quiet',
      '--offline',
      '--locked',
      '--manifest-path',
      fileURLToPath(new URL('../../../../rust/Cargo.toml', root)),
      '-p',
      'tmt-colab-model',
      '--example',
      example,
    ],
    { input: JSON.stringify(input), encoding: 'utf8', timeout: 60000, maxBuffer: 1024 * 1024 },
  );
  if (result.status !== 0 || result.error)
    throw new Error(`Rust interop failed: ${result.error ?? result.stderr}`);
  return JSON.parse(result.stdout);
}
const nativeEnvelope = native({
  seal: true,
  cases: [
    {
      envelope: fixtureEnvelope,
      public: fixture.public,
      secret: fixture.master,
      plaintext: fixture.plaintext,
      seed: fixture.seed,
    },
  ],
})[0];
const nativeAuthority = native(
  Object.fromEntries(
    ['seed', 'space', 'payload', 'page', 'wrap', 'recipientSeed', 'epochKey'].map((key) => [
      key,
      authority[key],
    ]),
  ),
  'browser_authority',
);
const modules = new Map();
for (const name of await readdir(new URL('src/', root))) {
  if (!name.endsWith('.ts')) continue;
  modules.set(
    '/' + name.replace(/\.ts$/, '.js'),
    ts.transpileModule(await readFile(new URL('src/' + name, root), 'utf8'), {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
    }).outputText,
  );
}
const server = createServer((req, res) => {
  const body =
    req.url === '/'
      ? '<!doctype html><script type="module">import * as c from "/index.js"; window.client=c;</script>'
      : modules.get(req.url);
  res.statusCode = body === undefined ? 404 : 200;
  res.setHeader('Content-Type', req.url === '/' ? 'text/html' : 'text/javascript');
  res.end(body ?? 'Missing test module');
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const results = [];
try {
  for (const [name, engine] of [
    ['chromium', chromium],
    ['firefox', firefox],
    ['webkit', webkit],
  ]) {
    let browser;
    try {
      // Optional local binary paths are explicit; absence/launch failures never become skips.
      browser = await engine.launch({
        headless: true,
        timeout: 30000,
        ...(process.env[`COLAB_${name.toUpperCase()}_EXECUTABLE`]
          ? { executablePath: process.env[`COLAB_${name.toUpperCase()}_EXECUTABLE`] }
          : {}),
      });
      const page = await browser.newPage();
      await page.goto(origin);
      await page.waitForFunction(() => window.client);
      const result = await page.evaluate(
        async ({ corpus, fixture, nativeEnvelope, authority, nativeAuthority, ownerCases }) => {
          const c = window.client;
          const hex = (s) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
          const same = (a, b) => c.equal(a, hex(b));
          const assert = (ok) => {
            if (!ok) throw new Error('Browser conformance check failed');
          };
          await c.probeCapabilities();
          const root = hex(authority.public);
          assert((await c.deriveSpaceId(root)) === authority.space);
          const genesis = await c.statement.Envelope.fromJson(
            c.text(JSON.stringify(authority.statement)),
          ).verifyNext(authority.space, root, null);
          assert(same(genesis.head.hash, authority.statementHash));
          for (const test of ownerCases) {
            const wire = test.envelope;
            assert(
              await c.strictVerify(
                root,
                c.binary(wire.signature, 64, 64),
                c.binary(wire.statement, 1024),
              ),
            );
            const envelope = c.statement.Envelope.fromJson(c.text(JSON.stringify(wire)));
            let accepted = false;
            try {
              await envelope.verifyNext(authority.space, root, genesis.head);
              accepted = true;
            } catch {}
            assert(accepted === test.accepted);
          }
          for (const test of authority.historyCases) {
            let accepted = false;
            try {
              await c.statement.Envelope.fromJson(c.text(JSON.stringify(test.envelope))).verifyNext(
                authority.space,
                root,
                genesis.head,
              );
              accepted = true;
            } catch {}
            assert(accepted === test.accepted);
          }
          let wrongHistoryOwner = false;
          try {
            await c.statement.Envelope.fromJson(
              c.text(JSON.stringify(authority.historyWrongOwner)),
            ).verifyNext(authority.space, root, genesis.head);
            wrongHistoryOwner = true;
          } catch {}
          assert(!wrongHistoryOwner);
          const join = authority.historyJoin;
          await c.statement.Envelope.fromJson(c.text(JSON.stringify(join.memberAdd))).verifyNext(
            authority.space,
            root,
            genesis.head,
          );
          const historyWrap = c.wrap.Envelope.fromJson(
            c.text(JSON.stringify(authority.forwardWrap)),
          );
          const historyKey = await crypto.subtle.importKey(
            'pkcs8',
            c.concat(hex('302e020100300506032b656e04220420'), hex(join.recipientSeed)),
            'X25519',
            false,
            ['deriveBits'],
          );
          const historyRecipient = await c.RecipientKey.fromHandle(
            historyKey,
            historyWrap.header().recipientKey,
          );
          assert(
            historyWrap.header().epoch === '63' && historyWrap.header().membershipRevision === '2',
          );
          assert(
            same(
              await historyWrap.open(historyWrap.header(), historyRecipient, root),
              authority.epochKey,
            ),
          );
          assert(
            join.wrapLists.length === 2 &&
              join.wrapLists[0].length === 512 &&
              join.wrapLists[1].length === 64,
          );
          let priorPage = '',
            priorEpoch = 0n;
          const historyPages = new Map();
          for (const list of join.wrapLists)
            for (const raw of list) {
              const w = c.wrap.Envelope.fromJson(c.text(raw)),
                h = w.header(),
                epoch = BigInt(h.epoch);
              assert(h.page > priorPage || (h.page === priorPage && epoch > priorEpoch));
              priorPage = h.page;
              priorEpoch = epoch;
              assert(
                epoch <= 64n &&
                  h.membershipRevision === '2' &&
                  h.recipientKind === 'member' &&
                  h.recipientId === '00000000-0000-4000-8000-000000000051',
              );
              historyPages.set(h.page, (historyPages.get(h.page) ?? 0) + 1);
              assert(same(await w.open(h, historyRecipient, root), authority.epochKey));
            }
          assert(historyPages.size === 9 && [...historyPages.values()].every((n) => n === 64));
          let head = null;
          for (const wire of nativeAuthority.statements)
            head = (
              await c.statement.Envelope.fromJson(c.text(JSON.stringify(wire))).verifyNext(
                authority.space,
                root,
                head,
              )
            ).head;
          assert(head.revision === 2n);
          const chain = c.certificate.Chain.fromJson(c.text(JSON.stringify(authority.chain)));
          assert(same(await chain.digest(), authority.chainDigest));
          await chain.verify(genesis.head.hash, chain.certificate(), root);
          const off = new Uint8Array(32);
          off[0] = 2;
          assert(c.validEdPoint(off));
          assert(
            !(await c.strictVerify(
              off,
              c.binary(authority.chain.issuerSignature, 64, 64),
              c.certificate.input(chain.certificate()),
            )),
          );
          let issued = false;
          try {
            await chain.verify(genesis.head.hash, chain.certificate(), off);
            issued = true;
          } catch {}
          assert(!issued);
          const independent = c.wrap.Envelope.fromJson(c.text(JSON.stringify(authority.wrap)));
          const x = await crypto.subtle.importKey(
            'pkcs8',
            c.concat(hex('302e020100300506032b656e04220420'), hex(authority.recipientSeed)),
            'X25519',
            false,
            ['deriveBits'],
          );
          const r = await c.RecipientKey.fromHandle(x, independent.header().recipientKey);
          for (const wire of [authority.wrap, ...nativeAuthority.wraps])
            assert(
              same(
                await c.wrap.Envelope.fromJson(c.text(JSON.stringify(wire))).open(
                  independent.header(),
                  r,
                  root,
                ),
                authority.epochKey,
              ),
            );
          const expected = independent.header(),
            ownerSnapshot = hex(authority.public);
          const opening = independent.open(expected, r, ownerSnapshot);
          expected.recipientKey.fill(0);
          ownerSnapshot.fill(0);
          expected.epoch = '2';
          assert(same(await opening, authority.epochKey));
          let wrong = false;
          try {
            await independent.open({ ...independent.header(), epoch: '2' }, r, root);
            wrong = true;
          } catch {}
          assert(!wrong);

          const rows = [];
          for (const v of corpus) {
            let raw = false;
            try {
              const key = await crypto.subtle.importKey('raw', hex(v.public), 'Ed25519', false, [
                'verify',
              ]);
              raw = await crypto.subtle.verify('Ed25519', key, hex(v.signature), hex(v.message));
            } catch {
              /* Recorded as false; native positive controls and probe still must pass. */
            }
            rows.push({
              name: v.name,
              raw,
              accepted: await c.strictVerify(hex(v.public), hex(v.signature), hex(v.message)),
            });
          }
          const h = hex(fixture.header),
            decoded = c.decodeHeader(h);
          const env = c.Envelope.fromJson(
            c.text(
              JSON.stringify({
                header: c.encodeBinary(h),
                nonce: c.encodeBinary(new Uint8Array(12)),
                ciphertext: c.encodeBinary(hex(fixture.ciphertext)),
                signature: c.encodeBinary(hex(fixture.signature)),
              }),
            ),
          );
          assert(
            same(
              await env.open(decoded.context, hex(fixture.master), hex(fixture.public)),
              fixture.plaintext,
            ),
          );
          assert(same(await env.hash(), fixture.envelopeHash));
          const native = c.Envelope.fromJson(c.text(JSON.stringify(nativeEnvelope)));
          assert(
            same(
              await native.open(decoded.context, hex(fixture.master), hex(fixture.public)),
              fixture.plaintext,
            ),
          );
          // Fixture import belongs only to this harness; no seed import exists in product APIs.
          const privateKey = await crypto.subtle.importKey(
            'pkcs8',
            c.concat(hex('302e020100300506032b657004220420'), hex(fixture.seed)),
            'Ed25519',
            false,
            ['sign'],
          );
          const master = hex(fixture.master),
            pt = hex(fixture.plaintext);
          const pending = c.Envelope.seal(decoded.context, master, privateKey, pt);
          master.fill(9);
          pt.fill(9);
          decoded.context.prevHash.fill(9);
          const a = await pending,
            original = c.decodeHeader(h).context;
          const b = await c.Envelope.seal(
            original,
            hex(fixture.master),
            privateKey,
            hex(fixture.plaintext),
          );
          assert(!c.equal(a.header(), b.header()));
          for (const e of [a, b])
            assert(
              same(
                await e.open(original, hex(fixture.master), hex(fixture.public)),
                fixture.plaintext,
              ),
            );
          const signer = hex(fixture.public),
            sig = hex(fixture.signature),
            msg = await c.signatureInput(h, hex(fixture.ciphertext));
          const verification = c.strictVerify(signer, sig, msg);
          signer.fill(0);
          sig.fill(0);
          msg.fill(0);
          assert(await verification);
          const recipient = await c.RecipientKey.generate();
          assert(!recipient.handle().extractable);
          let exported = false;
          try {
            await crypto.subtle.exportKey('pkcs8', recipient.handle());
            exported = true;
          } catch {}
          assert(!exported);
          await c.RecipientKey.fromHandle(recipient.handle(), recipient.publicKey());
          let low = false;
          try {
            const zero = await crypto.subtle.importKey(
              'raw',
              new Uint8Array(32),
              'X25519',
              false,
              [],
            );
            await crypto.subtle.deriveBits(
              { name: 'X25519', public: zero },
              recipient.handle(),
              256,
            );
            low = true;
          } catch {}
          assert(!low);
          const signin = {
            codeId: c.decodeText(c.fields(hex(fixture.signin), 8)[2]),
            space: original.space,
            device: original.authorDevice,
            signingKey: hex(fixture.public),
            encryptionKey: new Uint8Array(32).fill(7),
            nonce: Uint8Array.from({ length: 16 }, (_, i) => i),
          };
          assert(same(c.signinInput(signin), fixture.signin));
          assert(same(await c.signinProof(hex(fixture.code), signin), fixture.proof));
          assert(
            await c.verifySignin(
              hex(fixture.code),
              signin,
              hex(fixture.proof),
              hex(fixture.possessionSignature),
            ),
          );
          const management = {
            space: original.space,
            page: original.page,
            expectedRevision: '1',
            operationId: signin.codeId,
            operation: 'page.history',
            payload: hex(fixture.payload),
            senderDevice: original.authorDevice,
            issuedAt: 1790860000000,
            expiresAt: 1790860600000,
          };
          assert(same(await c.managementInput(management), fixture.management));
          let legacyAccepted = false;
          try {
            await c.managementInput({ ...management, operation: 'page.scripts' });
            legacyAccepted = true;
          } catch {}
          assert(!legacyAccepted, 'legacy management operation accepted');
          return {
            rows,
            checks: true,
            outgoing: [a, b].map((e) => ({
              envelope: c.decodeText(e.toJson()),
              public: fixture.public,
              secret: fixture.master,
              plaintext: fixture.plaintext,
            })),
          };
        },
        { corpus, fixture, nativeEnvelope, authority, nativeAuthority, ownerCases },
      );
      results.push({ engine: name, version: browser.version(), ...result });
    } catch (error) {
      results.push({ engine: name, error: String(error) });
    } finally {
      if (browser) await browser.close();
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}
const destination =
  process.env.COLAB_REPORT ?? fileURLToPath(new URL('differential-results.json', root));
await writeFile(destination, JSON.stringify(results, null, 2) + '\n');
validateReport(results, corpus);
const opened = native({
  seal: false,
  cases: results.flatMap((r) =>
    r.outgoing.map((v) => ({ ...v, envelope: JSON.parse(v.envelope) })),
  ),
});
if (opened.length !== 6 || opened.some((v) => v !== true))
  throw new Error('Incomplete browser-to-Rust interop');
console.log('Ciphertext interoperability: Rust to all3 browsers; all6 fresh browser seals to Rust');
console.log(
  ENGINES.map((engine) => `${engine}: 148 vectors, 9 positive controls, client checks passed`).join(
    '\n',
  ),
);
