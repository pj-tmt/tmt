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
const fixtureEnvelope = {
  header: Buffer.from(fixture.header, 'hex').toString('base64url'),
  nonce: Buffer.alloc(12).toString('base64url'),
  ciphertext: Buffer.from(fixture.ciphertext, 'hex').toString('base64url'),
  signature: Buffer.from(fixture.signature, 'hex').toString('base64url'),
};
function native(input) {
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
      'browser_conformance',
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
        async ({ corpus, fixture, nativeEnvelope }) => {
          const c = window.client;
          const hex = (s) => Uint8Array.from(s.match(/../g) ?? [], (n) => parseInt(n, 16));
          const same = (a, b) => c.equal(a, hex(b));
          const assert = (ok) => {
            if (!ok) throw new Error('Browser conformance check failed');
          };
          await c.probeCapabilities();
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
            operation: 'page.scripts',
            payload: hex(fixture.payload),
            senderDevice: original.authorDevice,
            issuedAt: 1790860000000,
            expiresAt: 1790860600000,
          };
          assert(same(await c.managementInput(management), fixture.management));
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
        { corpus, fixture, nativeEnvelope },
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
