// Standalone diagnostic, not conformance: no Colab module or admission helper runs in the page.
import { chromium, firefox, webkit } from 'playwright';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { engineDiagnostics } from './diagnostics.mjs';
import { parseEngines } from './gate.mjs';

const engines = parseEngines(process.argv.slice(2));
const raw = await readFile(
  new URL('../../../contracts/vectors/authority-v1.json', import.meta.url),
);
const authority = JSON.parse(raw);
const publicKey = Buffer.from(authority.public, 'hex');
const signature = Buffer.from(authority.chain.issuerSignature, 'base64url');
// Chain.decode requires certificate.input to reproduce these exact framed certificate bytes.
const message = Buffer.from(authority.chain.deviceCertificate, 'base64url');
const offCurve = Buffer.alloc(32);
offCurve[0] = 2;
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const packageIdentity = spawnSync(
  'dpkg-query',
  ['-W', '-f=${binary:Package}\t${Version}\n', 'libgcrypt20'],
  {
    encoding: 'utf8',
    timeout: 2000,
  },
);
const source = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8', timeout: 2000 });
const metadata = {
  source: source.status === 0 ? source.stdout.trim() : 'unavailable',
  node: process.version,
  playwright: createRequire(import.meta.url)('playwright/package.json').version,
  platform: process.platform,
  arch: process.arch,
  runnerImage: process.env.ImageVersion ?? null,
  libgcrypt: packageIdentity.status === 0 ? packageIdentity.stdout.trim() : 'unavailable',
  hashes: {
    harness: hash(await readFile(new URL(import.meta.url))),
    authority: hash(raw),
    publicKey: hash(publicKey),
    offCurve: hash(offCurve),
    signature: hash(signature),
    message: hash(message),
  },
};
const server = createServer((_, res) => {
  res.setHeader('Content-Type', 'text/html');
  res.end('<!doctype html><title>Raw Ed25519 diagnostic</title>');
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const results = [];
try {
  for (const name of engines) {
    for (const control of ['valid-root', 'off-curve']) {
      const engine = { chromium, firefox, webkit }[name];
      const diagnostics = engineDiagnostics();
      const entry = {
        engine: name,
        control,
        executable: engine.executablePath(),
        diagnostics: diagnostics.evidence,
      };
      results.push(entry);
      let browser;
      try {
        // A fresh browser/context per control prevents a crashed target from contaminating another.
        diagnostics.event('launch-started');
        browser = await engine.launch({ headless: true, timeout: 30000 });
        entry.version = browser.version();
        diagnostics.browser(browser);
        const page = await browser.newPage();
        diagnostics.page(page);
        await page.goto(`http://127.0.0.1:${server.address().port}`);
        const evaluation = page.evaluate(
          async ({ key, signature, message }) => {
            const progress = (state, id) =>
              console.log('colab-conformance:' + JSON.stringify([state, id]));
            progress('started', 'raw-import');
            let imported;
            try {
              imported = await crypto.subtle.importKey(
                'raw',
                new Uint8Array(key),
                'Ed25519',
                false,
                ['verify'],
              );
            } catch (error) {
              progress('completed', 'raw-import');
              return { outcome: 'import-rejected', error: String(error).slice(0, 512) };
            }
            progress('completed', 'raw-import');
            progress('started', 'raw-verify');
            let accepted;
            try {
              accepted = await crypto.subtle.verify(
                'Ed25519',
                imported,
                new Uint8Array(signature),
                new Uint8Array(message),
              );
            } catch (error) {
              progress('completed', 'raw-verify');
              return { outcome: 'verify-rejected', error: String(error).slice(0, 512) };
            }
            progress('completed', 'raw-verify');
            return { outcome: accepted ? 'accepted' : 'false' };
          },
          {
            key: [...(control === 'valid-root' ? publicKey : offCurve)],
            signature: [...signature],
            message: [...message],
          },
        );
        let timer;
        try {
          entry.result = await Promise.race([
            evaluation,
            new Promise((_, reject) => {
              timer = setTimeout(
                () => reject(new Error('Raw Ed25519 evaluation deadline exceeded')),
                30000,
              );
            }),
          ]);
        } finally {
          clearTimeout(timer);
        }
        entry.ok =
          control === 'valid-root'
            ? entry.result.outcome === 'accepted'
            : entry.result.outcome !== 'accepted';
      } catch (error) {
        diagnostics.failure(error);
        entry.error = String(error);
        entry.ok = false;
      } finally {
        if (browser) {
          try {
            await diagnostics.close(browser);
          } catch (error) {
            entry.cleanupError = String(error);
            entry.ok = false;
          }
        }
      }
    }
  }
} finally {
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
const report = { diagnosticOnly: true, metadata, results };
const out = process.env.COLAB_RAW_ED25519_REPORT ?? '/tmp/colab-raw-ed25519-report.json';
await writeFile(out, JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
if (results.some((entry) => !entry.ok)) process.exitCode = 1;
