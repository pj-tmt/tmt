import { chromium, firefox, webkit } from 'playwright';
import ts from 'typescript';
import { spawnSync } from 'node:child_process';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { parseEngines, validateReport } from './gate.mjs';
import { engineDiagnostics } from './diagnostics.mjs';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
const engines = parseEngines(process.argv.slice(2));
const root = new URL('../', import.meta.url),
  vectors = new URL('../../contracts/vectors/', root);
const corpus = (await readFile(new URL('ed25519-829.jsonl', vectors), 'utf8'))
  .trim()
  .split('\n')
  .map(JSON.parse);
const fixture = JSON.parse(await readFile(new URL('model-v1.json', vectors), 'utf8'));
const authority = JSON.parse(await readFile(new URL('authority-v1.json', vectors), 'utf8'));
const attachments = JSON.parse(await readFile(new URL('attachment-v1.json', vectors), 'utf8'));
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
const rustToolchain = process.env.COLAB_RUST_TOOLCHAIN ?? '+1.97.0';
const source = spawnSync('git', ['rev-parse', 'HEAD'], {
  cwd: fileURLToPath(root),
  encoding: 'utf8',
  timeout: 10000,
});
const rust = spawnSync('rustc', [rustToolchain, '--version'], {
  encoding: 'utf8',
  timeout: 10000,
});
const inputs = {};
for (const name of [
  'ed25519-829.jsonl',
  'model-v1.json',
  'authority-v1.json',
  'owner-member-v1.json',
  'attachment-v1.json',
])
  inputs[name] = createHash('sha256')
    .update(await readFile(new URL(name, vectors)))
    .digest('hex');
const metadata = {
  source: source.status === 0 ? source.stdout.trim() : 'unavailable',
  harnessSha256: createHash('sha256')
    .update(await readFile(fileURLToPath(import.meta.url)))
    .digest('hex'),
  node: process.version,
  playwright: createRequire(import.meta.url)('playwright/package.json').version,
  rustToolchain,
  rustc: rust.status === 0 ? rust.stdout.trim() : 'unavailable',
  platform: process.platform,
  arch: process.arch,
  runnerImage: process.env.ImageVersion ?? null,
  runnerOS: process.env.ImageOS ?? null,
  runId: process.env.GITHUB_RUN_ID ?? null,
  runAttempt: process.env.GITHUB_RUN_ATTEMPT ?? null,
  inputs,
};
try {
  for (const name of engines) {
    const engine = { chromium, firefox, webkit }[name];
    let browser;
    const diagnostics = engineDiagnostics();
    const entry = {
      engine: name,
      executable: process.env[`COLAB_${name.toUpperCase()}_EXECUTABLE`] ?? engine.executablePath(),
      diagnostics: diagnostics.evidence,
    };
    results.push(entry);
    try {
      diagnostics.event('launch-started');
      // Optional local binary paths are explicit; absence/launch failures never become skips.
      browser = await engine.launch({
        headless: true,
        timeout: 30000,
        ...(process.env[`COLAB_${name.toUpperCase()}_EXECUTABLE`]
          ? { executablePath: process.env[`COLAB_${name.toUpperCase()}_EXECUTABLE`] }
          : {}),
      });
      entry.version = browser.version();
      diagnostics.browser(browser);
      const page = await browser.newPage();
      diagnostics.page(page);
      await page.goto(origin);
      await page.waitForFunction(() => window.client);
      diagnostics.event('client-ready');
      const result = await page.evaluate(
        async ({
          engine,
          corpus,
          fixture,
          nativeEnvelope,
          authority,
          nativeAuthority,
          ownerCases,
          attachments,
        }) => {
          const c = window.client;
          const hex = (s) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
          const same = (a, b) => c.equal(a, hex(b));
          const assert = (ok, message = 'Browser conformance check failed') => {
            if (!ok) throw new Error(message);
          };
          // Synchronous markers add no awaited binding or change to crypto await ordering.
          const progress = (state, id) =>
            console.debug('colab-conformance:' + JSON.stringify([state, id]));
          progress('started', 'capabilities');
          await c.probeCapabilities();
          progress('completed', 'capabilities');
          let attachmentCases = 0;
          for (const test of attachments.cases) {
            if (test.operation === 'document' || test.operation === 'comment') continue;
            progress('started', 'attachment:' + test.name);
            let accepted = false;
            try {
              const a = c.attachment;
              if (test.operation === 'publication' || test.operation === 'publication-binding') {
                const record = a.decodeAttachmentPublication(c.text(test.input));
                if (test.operation === 'publication-binding')
                  await a.publicationMatchesDescriptor(
                    record,
                    a.decodeAttachment(c.text(test.descriptor)),
                  );
                else if (test.admit) assert(JSON.stringify(record) === test.canonical);
              } else if (test.operation === 'selector') {
                const selector = a.decodeAttachmentSelector(c.text(test.input));
                if (test.admit) assert(JSON.stringify(selector) === test.canonical);
              } else if (test.operation === 'manifest') {
                const m = a.decodeAttachmentManifest(c.text(test.input));
                if (test.inputBytes) {
                  assert(
                    c.equal(a.attachmentManifestInput(m), c.binary(test.inputBytes, 1024 * 1024)),
                  );
                  assert(same(await a.attachmentManifestHash(m), test.hash));
                }
              } else {
                const d = a.decodeAttachment(c.text(test.input));
                if (test.operation === 'open') {
                  const plain = await a.openAttachment(
                    d,
                    c.binary(test.payload, 1024 * 1024),
                    { ...a.attachmentContext(d), ...test.context },
                    c.binary(test.secret ?? attachments.secret, 32, 32),
                    c.binary(test.publicKey ?? attachments.publicKey, 32, 32),
                  );
                  assert(c.equal(plain, c.binary(attachments.plaintext, 1024)));
                } else if (test.admit) {
                  assert(c.decodeText(a.attachmentJson(d)) === test.canonical);
                  assert(c.equal(a.attachmentInput(d), c.binary(test.inputBytes, 1024 * 1024)));
                  assert(same(await a.attachmentHash(d), test.hash));
                }
              }
              accepted = true;
            } catch (error) {
              if (test.admit)
                throw new Error(`Positive attachment vector failed: ${test.name}`, {
                  cause: error,
                });
            }
            assert(accepted === test.admit, `Attachment vector: ${test.name}`);
            attachmentCases++;
            progress('completed', 'attachment:' + test.name);
          }
          progress('started', 'authority-genesis');
          const root = hex(authority.public);
          assert((await c.deriveSpaceId(root)) === authority.space);
          const genesis = await c.statement.Envelope.fromJson(
            c.text(JSON.stringify(authority.statement)),
          ).verifyNext(authority.space, root, null);
          assert(same(genesis.head.hash, authority.statementHash));
          progress('completed', 'authority-genesis');
          for (const test of ownerCases) {
            progress('started', 'owner-member:' + test.name);
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
            progress('completed', 'owner-member:' + test.name);
          }
          for (const test of authority.historyCases) {
            progress('started', 'history:' + test.name);
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
            progress('completed', 'history:' + test.name);
          }
          progress('started', 'history-owner-and-forward-wrap');
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
          progress('completed', 'history-owner-and-forward-wrap');
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
              progress('started', 'history-wrap:' + h.page + ':' + h.epoch);
              assert(same(await w.open(h, historyRecipient, root), authority.epochKey));
              progress('completed', 'history-wrap:' + h.page + ':' + h.epoch);
            }
          assert(historyPages.size === 9 && [...historyPages.values()].every((n) => n === 64));
          progress('started', 'native-authority-and-certificate');
          let head = null;
          for (const wire of nativeAuthority.statements) {
            progress('started', 'native-statement:' + nativeAuthority.statements.indexOf(wire));
            head = (
              await c.statement.Envelope.fromJson(c.text(JSON.stringify(wire))).verifyNext(
                authority.space,
                root,
                head,
              )
            ).head;
            progress('completed', 'native-statement:' + nativeAuthority.statements.indexOf(wire));
          }
          assert(head.revision === 2n);
          const chain = c.certificate.Chain.fromJson(c.text(JSON.stringify(authority.chain)));
          progress('started', 'certificate-digest');
          assert(same(await chain.digest(), authority.chainDigest));
          progress('completed', 'certificate-digest');
          progress('started', 'certificate-valid-root');
          await chain.verify(genesis.head.hash, chain.certificate(), root);
          progress('completed', 'certificate-valid-root');
          const off = new Uint8Array(32);
          off[0] = 2;
          assert(!c.validEdPoint(off));
          progress('started', 'off-curve-strict-verify');
          assert(
            !(await c.strictVerify(
              off,
              c.binary(authority.chain.issuerSignature, 64, 64),
              c.certificate.input(chain.certificate()),
            )),
          );
          progress('completed', 'off-curve-strict-verify');
          progress('started', 'off-curve-certificate');
          let issued = false;
          try {
            await chain.verify(genesis.head.hash, chain.certificate(), off);
            issued = true;
          } catch {}
          assert(!issued);
          progress('completed', 'off-curve-certificate');
          progress('completed', 'native-authority-and-certificate');
          progress('started', 'independent-wrap-and-aliasing');
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

          progress('completed', 'independent-wrap-and-aliasing');
          const rows = [];
          for (const v of corpus) {
            progress('started', 'ed25519:' + v.name);
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
            progress('completed', 'ed25519:' + v.name);
          }
          progress('started', 'native-envelope-and-root-handle');
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
          const rootHandle = await crypto.subtle.importKey(
            'raw',
            hex(fixture.master),
            'HKDF',
            false,
            ['deriveBits'],
          );
          assert(rootHandle.extractable === false, 'HKDF root is extractable');
          assert(
            same(
              await env.open(decoded.context, rootHandle, hex(fixture.public)),
              fixture.plaintext,
            ),
          );
          progress('completed', 'native-envelope-and-root-handle');
          const frozenSeals = [];
          const entropy = Object.getOwnPropertyDescriptor(crypto, 'getRandomValues');
          // Harness-only entropy control; no caller-selected ID API is added.
          try {
            Object.defineProperty(crypto, 'getRandomValues', {
              configurable: true,
              value: (bytes) => {
                assert(bytes instanceof Uint8Array && bytes.length === 32);
                bytes.set(hex(decoded.objectId));
                return bytes;
              },
            });
            for (const root of [hex(fixture.master), rootHandle]) {
              progress('started', 'frozen-seal:' + frozenSeals.length);
              const sealed = await c.Envelope.seal(
                decoded.context,
                root,
                privateKey,
                hex(fixture.plaintext),
              );
              assert(c.equal(sealed.header(), env.header()), 'frozen root-path header differs');
              assert(
                c.equal(sealed.ciphertext(), env.ciphertext()),
                'frozen root-path ciphertext differs',
              );
              assert(
                await c.strictVerify(
                  hex(fixture.public),
                  sealed.signature(),
                  await c.signatureInput(sealed.header(), sealed.ciphertext()),
                ),
                'frozen root-path signature is invalid',
              );
              assert(
                same(
                  await sealed.open(decoded.context, rootHandle, hex(fixture.public)),
                  fixture.plaintext,
                ),
              );
              // WebKit's native signer uses randomized Ed25519 signatures. Derivation is exact;
              // full frozen-envelope equality belongs to the deterministic engines and Node.
              if (engine !== 'webkit') {
                assert(c.equal(sealed.toJson(), env.toJson()), 'frozen root-path envelope differs');
                assert(
                  same(await sealed.hash(), fixture.envelopeHash),
                  'frozen root-path hash differs',
                );
              }
              progress('completed', 'frozen-seal:' + frozenSeals.length);
              frozenSeals.push(sealed);
            }
          } finally {
            if (entropy) Object.defineProperty(crypto, 'getRandomValues', entropy);
            else delete crypto.getRandomValues;
          }
          progress('started', 'fresh-seals-and-aliasing');
          const master = hex(fixture.master),
            pt = hex(fixture.plaintext);
          const pending = c.Envelope.seal(decoded.context, master, privateKey, pt);
          master.fill(9);
          pt.fill(9);
          decoded.context.prevHash.fill(9);
          const a = await pending,
            original = c.decodeHeader(h).context;
          const b = await c.Envelope.seal(original, rootHandle, privateKey, hex(fixture.plaintext));
          assert(!c.equal(a.header(), b.header()));
          for (const e of [a, b])
            assert(
              same(await e.open(original, rootHandle, hex(fixture.public)), fixture.plaintext),
            );
          const signer = hex(fixture.public),
            sig = hex(fixture.signature),
            msg = await c.signatureInput(h, hex(fixture.ciphertext));
          const verification = c.strictVerify(signer, sig, msg);
          signer.fill(0);
          sig.fill(0);
          msg.fill(0);
          assert(await verification);
          progress('completed', 'fresh-seals-and-aliasing');
          progress('started', 'recipient-nonextractability-and-low-order');
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
          progress('completed', 'recipient-nonextractability-and-low-order');
          progress('started', 'signin-and-management');
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
          progress('completed', 'signin-and-management');
          return {
            rows,
            checks: true,
            attachmentCases,
            outgoing: [...frozenSeals, a, b].map((e) => ({
              envelope: c.decodeText(e.toJson()),
              public: fixture.public,
              secret: fixture.master,
              plaintext: fixture.plaintext,
            })),
          };
        },
        {
          engine: name,
          corpus,
          fixture,
          nativeEnvelope,
          authority,
          nativeAuthority,
          ownerCases,
          attachments,
        },
      );
      diagnostics.event('evaluation-completed');
      Object.assign(entry, result);
    } catch (error) {
      diagnostics.failure(error);
      entry.error = String(error);
    } finally {
      if (browser) {
        try {
          await diagnostics.close(browser);
        } catch (error) {
          entry.error ??= String(error); // Cleanup failures cannot satisfy the normal report gate.
        }
      }
    }
  }
} finally {
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
const destination =
  process.env.COLAB_REPORT ?? fileURLToPath(new URL('differential-results.json', root));
await writeFile(destination, JSON.stringify({ metadata, engines, results }, null, 2) + '\n');
validateReport(results, corpus, engines);
const expectedAttachmentCases = attachments.cases.filter(
  (c) => c.operation !== 'document' && c.operation !== 'comment',
).length;
if (results.some((r) => r.attachmentCases !== expectedAttachmentCases))
  throw new Error('Incomplete attachment browser corpus');
// Keep each engine's four seals within the native example's bounded input batch.
const opened = results.flatMap((r) =>
  native({
    seal: false,
    cases: r.outgoing.map((v) => ({ ...v, envelope: JSON.parse(v.envelope) })),
  }),
);
if (opened.length !== engines.length * 4 || opened.some((v) => v !== true))
  throw new Error('Incomplete browser-to-Rust interop');
console.log(`Selected engines: ${engines.join(', ')}`);
console.log(
  `Ciphertext interoperability: Rust to all${engines.length} browsers; all${opened.length} fresh browser seals to Rust`,
);
console.log(
  engines
    .map((engine) => `${engine}: 148 vectors, 9 positive controls, client checks passed`)
    .join('\n'),
);
