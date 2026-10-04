import { execFile, spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer, request, type Server } from 'node:http';
import { createConnection, type AddressInfo } from 'node:net';
import type { Duplex } from 'node:stream';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { expect, test, type BrowserContext } from '@playwright/test';

const execute = promisify(execFile);
const mount = '/r/abcd/x/colab/';
const app = fileURLToPath(new URL('../dist/', import.meta.url));
const binary = process.env.COLAB_SERVE_EXECUTABLE;
const producer = process.env.COLAB_PAGE_FIXTURE_EXECUTABLE;
interface Fixture {
  pageId: string;
  deviceId: string;
  context: string;
  signPublic: string;
  encPublic: string;
}
/** Remote pairing is represented by public test seeds. Actual registration proofs,
 * certificates, wraps, source updates, socket transport and browser decoding run. */
const sdk = `
const bytes = (n) => new Uint8Array(n);
const seed = (prefix,n) => bytes([...prefix,...Array(32).fill(n)]);
const signPrefix = [48,46,2,1,0,48,5,6,3,43,101,112,4,34,4,32];
const encode = (b) => btoa(String.fromCharCode(...b)).replaceAll('+','-').replaceAll('/','_').replace(/=+$/,'');
export async function reopenSession(){await window.fixtureKeys;}
export async function certifyKey(purpose,publicKey){
  const issuedAtMs = Date.now();
  const fields = ['tmt-ext-cert-v1','colab',purpose,publicKey,String(issuedAtMs)]
    .map(v => typeof v === 'string' ? new TextEncoder().encode(v) : v);
  const input = new Uint8Array(fields.reduce((n,v) => n+4+v.length,0));
  let offset = 0;
  for(const field of fields){new DataView(input.buffer).setUint32(offset,field.length);offset+=4;input.set(field,offset);offset+=field.length;}
  const signer = await crypto.subtle.importKey('pkcs8',seed(signPrefix,9),'Ed25519',false,['sign']);
  const signature = bytes(await crypto.subtle.sign('Ed25519',signer,input));
  return {publicKey:encode(publicKey),issuedAtMs,signature:encode(signature)};
}`;
async function keys(context: BrowserContext, fixture: Fixture) {
  await context.addInitScript(({ deviceId, signPublic, encPublic }) => {
    if (window !== window.top) return;
    const decode = (v: string) =>
      Uint8Array.from(atob(v.replaceAll('-', '+').replaceAll('_', '/')), (c) => c.charCodeAt(0));
    const seed = (oid: number, n: number) =>
      new Uint8Array([
        48,
        46,
        2,
        1,
        0,
        48,
        5,
        6,
        3,
        43,
        101,
        oid,
        4,
        34,
        4,
        32,
        ...Array<number>(32).fill(n),
      ]);
    (window as unknown as { fixtureKeys: Promise<void> }).fixtureKeys = (async () => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const open = indexedDB.open('tmt-colab', 1);
        open.onupgradeneeded = () => open.result.createObjectStore('keys');
        open.onsuccess = () => resolve(open.result);
        open.onerror = () => reject(open.error);
      });
      try {
        const sign = await crypto.subtle.importKey('pkcs8', seed(112, 10), 'Ed25519', false, [
          'sign',
        ]);
        const enc = await crypto.subtle.importKey('pkcs8', seed(110, 17), 'X25519', false, [
          'deriveBits',
        ]);
        await new Promise<void>((resolve, reject) => {
          const tx = db.transaction('keys', 'readwrite');
          tx.objectStore('keys').put(
            { sign, signPublic: decode(signPublic), enc, encPublic: decode(encPublic) },
            `keys:${deviceId}`,
          );
          tx.oncomplete = () => resolve();
          tx.onabort = () => reject(tx.error);
        });
      } finally {
        db.close();
      }
    })();
  }, fixture);
}
async function native() {
  if (!binary || !producer)
    throw new Error(
      'Set COLAB_SERVE_EXECUTABLE and COLAB_PAGE_FIXTURE_EXECUTABLE for native CLI acceptance',
    );
  await Promise.all([access(binary), access(producer), access(app + '/index.html')]);
  const root = await mkdtemp('/tmp/colab1438-browser-');
  const env = { ...process.env, TMT_EXECUTABLE: process.execPath };
  let child: ChildProcess | undefined,
    door: Server | undefined,
    socketPath = '';
  let exited: Promise<unknown> | undefined;
  const tunnels = new Set<Duplex>();
  const stop = async () => {
    for (const tunnel of tunnels) tunnel.destroy();
    if (child?.pid) {
      const pid = child.pid;
      if (child.exitCode === null && child.signalCode === null) {
        const stopped = once(child, 'exit', { signal: AbortSignal.timeout(3000) });
        child.kill('SIGTERM');
        try {
          await stopped;
        } catch (error) {
          child.kill('SIGKILL');
          await exited;
          throw error;
        }
      }
      await exited;
      expect(child.exitCode).toBe(0);
      expect(() => process.kill(pid, 0)).toThrow();
      child = undefined;
    }
  };
  const close = async () => {
    try {
      try {
        for (const tunnel of tunnels) tunnel.destroy();
        if (door) {
          const closed = once(door, 'close');
          door.close();
          door.closeAllConnections();
          await closed;
          door = undefined;
        }
      } finally {
        await stop();
      }
      if (socketPath) await expect(access(socketPath)).rejects.toThrow();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  };
  try {
    await execute(producer, ['--exact', 'seed_native_page', '--ignored'], {
      env: { ...env, COLAB_PAGE_FIXTURE_ROOT: root },
      timeout: 10000,
    });
    const fixture = JSON.parse(await readFile(root + '/browser-fixture.json', 'utf8')) as Fixture;
    await writeFile(
      root + '/api',
      `const fs=require('node:fs'); const r=JSON.parse(fs.readFileSync(0,'utf8')); if(r.version!==1||r.operation!=='storage.root')process.exit(9);console.log(JSON.stringify({dataRoot:${JSON.stringify(root)}}));`,
    );
    child = spawn(binary, ['serve', '--json', '--app-dir', app], {
      cwd: root,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stderr = '';
    child.stderr!.on('data', (bytes: Buffer) => {
      stderr = (stderr + bytes.toString()).slice(-8192);
    });
    exited = once(child, 'exit');
    const lines = createInterface({ input: child.stdout! });
    try {
      const [line] = await Promise.race([
        once(lines, 'line', { signal: AbortSignal.timeout(5000) }),
        exited.then(() => {
          throw new Error(`Serve quit before readiness: ${stderr}`);
        }),
      ]);
      const ready = JSON.parse(String(line)) as { socket: string; state: string };
      expect(ready.state).toBe('mounted');
      socketPath = ready.socket;
      await access(socketPath);
    } finally {
      lines.close();
    }
    door = createServer((incoming, outgoing) => {
      if (incoming.url === '/sdk/remote-v1.js') {
        outgoing.writeHead(200, { 'Content-Type': 'text/javascript' });
        outgoing.end(sdk);
        return;
      }
      const below = incoming.url?.startsWith(mount)
        ? '/' + incoming.url.slice(mount.length)
        : incoming.url;
      const forward = request(
        {
          socketPath,
          path: below,
          method: incoming.method,
          headers: { ...incoming.headers, 'tmt-device-context': fixture.context },
        },
        (reply) => {
          outgoing.writeHead(reply.statusCode!, reply.headers);
          reply.pipe(outgoing);
        },
      );
      forward.on('error', (error) => outgoing.destroy(error));
      incoming.pipe(forward);
    });
    door.on('upgrade', (incoming, downstream, head) => {
      const upstream = createConnection(socketPath);
      for (const tunnel of [downstream, upstream]) {
        tunnels.add(tunnel);
        tunnel.on('close', () => tunnels.delete(tunnel));
        tunnel.on('error', () => {
          upstream.destroy();
          downstream.destroy();
        });
      }
      upstream.on('connect', () => {
        const path = '/' + incoming.url!.slice(mount.length);
        const headers = { ...incoming.headers, 'tmt-device-context': fixture.context };
        upstream.write(
          `GET ${path} HTTP/1.1\r\n${Object.entries(headers)
            .map(([key, value]) => `${key}: ${value}\r\n`)
            .join('')}\r\n`,
        );
        if (head.length) upstream.write(head);
        downstream.pipe(upstream);
        upstream.pipe(downstream);
      });
    });
    door.listen(0, '127.0.0.1');
    await once(door, 'listening');
    const cli = (args: string[]) =>
      execute(binary, args, { cwd: root, env, timeout: 10000, maxBuffer: 4 * 1024 * 1024 });
    const write = async (source: string, revision?: string) => {
      await writeFile(root + '/source.html', source);
      return cli([
        'page',
        'write',
        fixture.pageId,
        '--file',
        root + '/source.html',
        '--json',
        ...(revision ? ['--expected-revision', revision] : []),
      ]);
    };
    return {
      fixture,
      cli,
      write,
      stop,
      close,
      origin: `http://127.0.0.1:${(door.address() as AddressInfo).port}`,
    };
  } catch (error) {
    await close();
    throw error;
  }
}
test('CLI writes reach a live native browser, preserve title and refuse a stale browser base', async ({
  page,
  context,
}) => {
  const server = await native();
  try {
    await keys(context, server.fixture);
    await page.goto(server.origin + mount);
    await page.getByRole('link', { name: new RegExp(server.fixture.pageId) }).click();
    await expect(page.getByRole('heading', { name: 'CLI page', exact: true })).toBeVisible();
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Before CLI' }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    const initial = JSON.parse(
      (await server.cli(['page', 'read', server.fixture.pageId, '--json'])).stdout,
    ) as { revision: string };
    const first = JSON.parse(
      (await server.write('<h1>After CLI 🐈</h1>', initial.revision)).stdout,
    ) as { streamId: string };
    expect(first.streamId).not.toBe(server.fixture.deviceId);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'After CLI 🐈' }),
    ).toBeVisible();
    await expect(page.getByRole('textbox', { name: 'Source', exact: true })).toHaveValue(
      '<h1>After CLI 🐈</h1>',
    );
    await expect(page.getByRole('heading', { name: 'CLI page', exact: true })).toBeVisible();
    const chunked = '<h1>Chunked CLI</h1><p>' + 'x'.repeat(48 * 1024) + '</p>';
    await server.write(chunked);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Chunked CLI' }),
    ).toBeVisible();
    await expect(page.getByRole('textbox', { name: 'Source', exact: true })).toHaveValue(chunked);
    const old = JSON.parse(
      (await server.cli(['page', 'read', server.fixture.pageId, '--json'])).stdout,
    ) as { revision: string };
    await page.getByRole('textbox', { name: 'Source', exact: true }).fill('<h1>Browser edit</h1>');
    await page.getByRole('button', { name: 'Save source' }).click();
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Browser edit' }),
    ).toBeVisible();
    await expect(server.write('<h1>Stale CLI</h1>', old.revision)).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining('COLAB_STALE_BASE'),
    });
    const actual = JSON.parse(
      (await server.cli(['page', 'read', server.fixture.pageId, '--json'])).stdout,
    ) as { source: string; title: string };
    expect(actual).toMatchObject({ source: '<h1>Browser edit</h1>', title: 'CLI page' });
    await page.goto('about:blank');
    await server.stop();
    await server.write('<h1>Offline again</h1>');
    expect(
      JSON.parse((await server.cli(['page', 'read', server.fixture.pageId, '--json'])).stdout)
        .source,
    ).toBe('<h1>Offline again</h1>');
  } finally {
    await page.goto('about:blank');
    await server.close();
  }
});
