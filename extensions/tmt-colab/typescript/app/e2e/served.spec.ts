import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, readdir, rm, writeFile, access, mkdir } from 'node:fs/promises';
import { createServer, request, type Server } from 'node:http';
import { type AddressInfo } from 'node:net';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { writeExecutable } from '../../../../../typescript/test/support/executable-fixture.mjs';

const checkout = fileURLToPath(new URL('../dist/', import.meta.url));
const app = process.env.COLAB_SERVE_APP_DIR ?? checkout;
const embedded = process.env.COLAB_SERVE_EMBEDDED === '1';
const binary =
  process.env.COLAB_SERVE_EXECUTABLE ??
  fileURLToPath(new URL('../../../../../rust/target/debug/tmt-colab', import.meta.url));
const mount = '/r/abcd/x/colab/';
const owner = JSON.stringify({
  owner: true,
  deviceId: '00000000-0000-4000-8000-000000000004',
  name: 'Chromium fixture',
});

/** Only test forwarding and core storage-root discovery are simulated. All app
 * HTML/JS/CSS responses come from the foreground executable's real Unix socket. */
async function serve(explicit: boolean) {
  await access(binary);
  const root = await mkdtemp('/tmp/tmt-1253-browser-');
  let child: ChildProcess | undefined,
    door: Server | undefined,
    exited: Promise<unknown> | undefined,
    socket = '';
  const stopChild = async () => {
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
        if (door) {
          const closed = once(door, 'close');
          door.close();
          door.closeAllConnections();
          await closed;
          door = undefined;
        }
      } finally {
        await stopChild();
      }
      if (socket) await expect(access(socket)).rejects.toThrow();
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  };
  try {
    // Use the existing Node executable as the fixture, avoiding freshly written
    // executable scripts and their ETXTBSY publication race.
    await writeFile(
      `${root}/api`,
      `const fs = require('node:fs');
const request = JSON.parse(fs.readFileSync(0, 'utf8'));
if (request.version !== 1 || request.operation !== 'storage.root') process.exit(9);
console.log(JSON.stringify({dataRoot: ${JSON.stringify(root)}}));`,
    );
    let executable = binary;
    if (embedded) {
      // The existing isolated publisher closes all writable handles before exec.
      await mkdir(`${root}/install`);
      executable = `${root}/install/tmt-colab`;
      writeExecutable(executable, await readFile(binary));
      await expect(access(`${root}/install/colab-app`)).rejects.toThrow();
      if (!explicit) await expect(access(checkout)).rejects.toThrow();
    }
    child = spawn(executable, ['serve', '--json', ...(explicit ? ['--app-dir', app] : [])], {
      cwd: root,
      env: { ...process.env, TMT_EXECUTABLE: process.execPath },
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
          throw new Error(`Colab quit before readiness: ${stderr}`);
        }),
      ]);
      const descriptor = JSON.parse(String(line)) as { socket: string; state: string };
      expect(descriptor.state).toBe('mounted');
      socket = descriptor.socket;
      await access(socket);
    } finally {
      lines.close();
    }
    door = createServer((incoming, outgoing) => {
      const path = incoming.url ?? '/';
      const below = path.startsWith(mount) ? '/' + path.slice(mount.length) : path;
      const forward = request(
        {
          socketPath: socket,
          path: below,
          method: incoming.method,
          headers: incoming.headers.cookie === 'owner=1' ? { 'tmt-device-context': owner } : {},
        },
        (reply) => {
          outgoing.writeHead(reply.statusCode!, reply.headers);
          reply.pipe(outgoing);
        },
      );
      forward.on('error', (error) => outgoing.destroy(error));
      incoming.pipe(forward);
    });
    door.listen(0, '127.0.0.1');
    await once(door, 'listening');
    const origin = `http://127.0.0.1:${(door.address() as AddressInfo).port}`;
    return { origin, close };
  } catch (error) {
    await close();
    throw error;
  }
}

for (let run = 1; run <= 2; run++) {
  test(`built app through the real socket: nested mount, CSP, bytes and cleanup (${run})`, async ({
    page,
    context,
  }) => {
    // Prove override plus checkout default, or relocated embedded default when selected.
    const server = await serve(run === 1);
    let capture: Server | undefined;
    const requests: string[] = [];
    page.on('request', (request) => requests.push(request.url()));
    try {
      await context.addCookies([{ name: 'owner', value: '1', url: server.origin }]);
      const response = await page.goto(server.origin + mount);
      expect(response!.status()).toBe(200);
      expect(await response!.body()).toEqual(await readFile(app + '/index.html'));
      const appPolicy = response!.headers()['content-security-policy'];
      expect(appPolicy).toContain("script-src 'self'; style-src 'self'");
      expect(appPolicy).not.toContain('unsafe-inline');
      const renderer = await context.request.get(server.origin + mount + 'renderer.html');
      expect(renderer.status()).toBe(200);
      expect(await renderer.body()).toEqual(await readFile(app + '/renderer.html'));
      expect(renderer.headers()['content-security-policy']).toContain('sandbox allow-scripts');
      expect(renderer.headers()['content-security-policy']).toContain("connect-src 'none'");
      // The renderer is public static bytes: the read-only entry frames pages with it too.
      const anonymousRenderer = await fetch(server.origin + mount + 'renderer.html');
      expect(anonymousRenderer.status).toBe(200);
      expect(anonymousRenderer.headers.get('content-security-policy')).toContain(
        'sandbox allow-scripts',
      );
      // The test door does not serve Remote's SDK. Its visible blocking state
      // proves the actual compiled mounted entry ran, rather than a preview.
      await expect(page.getByRole('alert')).toContainText('Could not open this paired space');
      await expect(page.getByRole('button', { name: 'Reload' })).toBeVisible();
      if (run === 1) await page.screenshot({ path: '/tmp/tmt-1253-mounted.png', fullPage: true });
      // An inline handler injected into trusted chrome must not execute, while
      // a normal addEventListener is a valid positive control on the same button.
      const injected = await page.evaluate(async () => {
        const button = document.createElement('button');
        button.setAttribute('onclick', "document.body.dataset.inlineExecuted = 'yes'");
        button.addEventListener('click', () => {
          button.dataset.control = 'yes';
        });
        document.body.append(button);
        button.click();
        const result = {
          inline: document.body.dataset.inlineExecuted,
          control: button.dataset.control,
        };
        button.remove();
        return result;
      });
      expect(injected).toEqual({ inline: undefined, control: 'yes' });
      const style = await page.evaluate(() => getComputedStyle(document.body).fontSize);
      expect(style).toBe('14px');
      expect(requests.some((url) => url.startsWith(server.origin + mount + 'assets/'))).toBe(true);
      const files = await readdir(app + '/assets');
      expect(files.some((name) => name.endsWith('.css'))).toBe(true);
      expect(files.some((name) => name.endsWith('.js'))).toBe(true);
      for (const name of files) {
        const asset = await context.request.get(server.origin + mount + 'assets/' + name);
        expect(asset.status()).toBe(200);
        expect(await asset.body()).toEqual(await readFile(app + '/assets/' + name));
        if (name.endsWith('.js'))
          expect(asset.headers()['content-type']).toBe('text/javascript; charset=utf-8');
        if (name.endsWith('.css'))
          expect(asset.headers()['content-type']).toBe('text/css; charset=utf-8');
      }
      const roots = await readdir(app);
      for (const name of ['THIRD-PARTY-NOTICES.txt', 'renderer.html']) {
        if (!roots.includes(name)) continue;
        const asset = await context.request.get(server.origin + mount + name);
        expect(asset.status()).toBe(200);
        expect(await asset.body()).toEqual(await readFile(app + '/' + name));
        expect(asset.headers()['content-type']).toBe(
          name.endsWith('.html') ? 'text/html; charset=utf-8' : 'text/plain; charset=utf-8',
        );
      }
      expect(requests.every((url) => new URL(url).origin === server.origin)).toBe(true);
      // Owner files stay owner-only; only the exact public allowlist is anonymous.
      const publicFiles = ['reader.js', 'reader.css', 'reader-fold.js', 'recovery.js'];
      const anonymous = await fetch(
        server.origin + mount + 'assets/' + files.find((name) => !publicFiles.includes(name)),
      );
      expect(anonymous.status).toBe(403);
      expect((await fetch(server.origin + mount + 'index.html')).status).toBe(403);
      expect((await fetch(server.origin + mount + 'reader.html')).status).toBe(403);
      const publicRecovery = await fetch(server.origin + mount + 'assets/recovery.js');
      expect(publicRecovery.status).toBe(200);
      expect(Buffer.from(await publicRecovery.arrayBuffer())).toEqual(
        await readFile(app + '/assets/recovery.js'),
      );
      const privatePage = await fetch(server.origin + mount);
      expect(await privatePage.text()).toContain('This colab space is private');

      // The compiled public entry really executes under native private-guidance
      // CSP. This door has no SDK/key: failure reveals guidance, and reload
      // retains the marker instead of repeatedly loading the SDK/reopening.
      await context.clearCookies();
      await page.goto(server.origin + mount);
      await expect(page.locator('#colab-guidance')).toBeVisible();
      const recoverySdkRequests = requests.filter(
        (url) => new URL(url).pathname === '/sdk/remote-v1.js',
      ).length;
      await page.reload();
      await expect(page.locator('#colab-guidance')).toBeVisible();
      expect(requests.filter((url) => new URL(url).pathname === '/sdk/remote-v1.js')).toHaveLength(
        recoverySdkRequests,
      );
      expect(
        await page.evaluate((path) => sessionStorage.getItem(`colab-recovery:${path}`), mount),
      ).toBe('attempted');
      if (run === 1)
        await page.screenshot({
          path: '/private/tmp/colab-1110-design/colab-guidance.png',
          fullPage: true,
        });
      await context.addCookies([{ name: 'owner', value: '1', url: server.origin }]);

      // CSP sandbox also protects a top-level renderer, without an iframe attribute.
      await page.goto(server.origin + mount + 'renderer.html');
      const isolation = await page.evaluate(() => {
        const denied = (read: () => unknown) => {
          try {
            read();
            return false;
          } catch {
            return true;
          }
        };
        return {
          storage: denied(() => localStorage.getItem('secret')),
          cookie: denied(() => document.cookie),
          indexedDB: denied(() => indexedDB.open('keys')),
        };
      });
      expect(isolation).toEqual({ storage: true, cookie: true, indexedDB: true });

      // The same served build at the test door's root selects its sample adapter,
      // exercising the opaque renderer beneath the production response CSP.
      await page.goto(server.origin + '/');
      await page.getByRole('link', { name: /A shared page/ }).click();
      const frame = page.frameLocator('iframe');
      await expect(frame.getByRole('button', { name: 'Try the page: 0' })).toBeVisible();
      await frame.getByRole('button', { name: 'Try the page: 0' }).click();
      await expect(frame.getByRole('button', { name: 'Try the page: 1' })).toBeVisible();
      expect(
        await frame.getByRole('heading').evaluate((heading) => getComputedStyle(heading).fontSize),
      ).toBe('34px');
      await expect(page.locator('iframe')).toHaveAttribute('sandbox', 'allow-scripts');
      if (run === 1) await page.screenshot({ path: '/tmp/tmt-1253-renderer.png', fullPage: true });
      expect(requests.every((url) => new URL(url).origin === server.origin)).toBe(true);

      const captured: string[] = [];
      capture = createServer((incoming, outgoing) => {
        captured.push(incoming.url!);
        outgoing.setHeader('Access-Control-Allow-Origin', '*');
        outgoing.end('capture control');
      });
      capture.listen(0, '127.0.0.1');
      await once(capture, 'listening');
      const external = `http://127.0.0.1:${(capture.address() as AddressInfo).port}`;
      // The same browser reaches the capture from the dev page. Only the native
      // app's response CSP changes for the negative; a blind server cannot pass.
      await page.goto('/');
      await page.evaluate((url) => fetch(url + '/positive-control'), external);
      expect(captured).toEqual(['/positive-control']);
      await page.goto(server.origin + mount);
      await expect(page.getByRole('alert')).toContainText('Could not open this paired space');
      const denied = await page.evaluate(async (url) => {
        try {
          await fetch(url + '/blocked', { signal: AbortSignal.timeout(2000) });
          return false;
        } catch {
          return true;
        }
      }, external);
      expect(denied).toBe(true);
      expect(captured).toEqual(['/positive-control']);
    } finally {
      try {
        await page.goto('about:blank');
      } finally {
        try {
          await server.close();
        } finally {
          if (capture) {
            const closed = once(capture, 'close');
            capture.close();
            capture.closeAllConnections();
            await closed;
          }
        }
      }
    }
  });
}

// #1545: the public read-only entry is served to an unpaired browser as static bytes only. The
// real executable answers; the test door forwards without owner context.
test('public reader entry: exact static bytes, link fragment removed, and no access without a grant', async ({
  page,
  context,
}) => {
  const server = await serve(false);
  try {
    const entry = await fetch(server.origin + mount + 'read');
    expect(entry.status).toBe(200);
    expect(Buffer.from(await entry.arrayBuffer())).toEqual(await readFile(app + '/reader.html'));
    expect(entry.headers.get('content-security-policy')).toContain("script-src 'self'");
    expect(entry.headers.get('cache-control')).toBe('no-store');
    for (const name of ['reader.js', 'reader.css', 'reader-fold.js']) {
      const asset = await fetch(server.origin + mount + 'assets/' + name);
      expect(asset.status).toBe(200);
      expect(Buffer.from(await asset.arrayBuffer())).toEqual(
        await readFile(app + '/assets/' + name),
      );
    }
    expect((await fetch(server.origin + mount + 'renderer.html')).status).toBe(200);
    // Nothing else of the owner surface opens without owner context.
    for (const path of ['index.html', 'api/pages', 'api/session', 'THIRD-PARTY-NOTICES.txt'])
      expect((await fetch(server.origin + mount + path)).status).toBe(403);

    const requests: { url: string; body: string | null }[] = [];
    page.on('request', (request) =>
      requests.push({ url: request.url(), body: request.postData() }),
    );
    // A malformed link explains itself, and its fragment leaves the address bar first.
    await page.goto(server.origin + mount + 'read#v=1&seed=not-a-seed');
    await expect(page.getByRole('alert')).toContainText('incomplete or malformed');
    expect(await page.evaluate(() => location.hash)).toBe('');
    // A well-formed link for a space this server does not hold has no grant: access ended.
    const seed = Buffer.alloc(32, 7).toString('base64url');
    const statement = Buffer.alloc(32, 9).toString('base64url');
    const link = [
      'v=1',
      'space=' + 'a'.repeat(32),
      'page=10000000-0000-4000-8000-000000000001',
      'link=20000000-0000-4000-8000-000000000001',
      'rev=4',
      'st=' + statement,
      'seed=' + seed,
    ].join('&');
    await page.goto('about:blank'); // A hash-only change would not reload the entry.
    await page.goto(server.origin + mount + 'read#' + link);
    await expect(page.getByRole('heading', { name: 'Access ended' })).toBeVisible();
    expect(await page.evaluate(() => location.hash)).toBe('');
    // The seed is in no request, and the reader stored nothing in the browser.
    expect(requests.filter((r) => r.url.includes(seed) || (r.body ?? '').includes(seed))).toEqual(
      [],
    );
    expect(
      await page.evaluate(async () => ({
        local: localStorage.length,
        session: sessionStorage.length,
        databases: (await indexedDB.databases()).length,
        cookie: document.cookie,
      })),
    ).toEqual({ local: 0, session: 0, databases: 0, cookie: '' });
    await context.clearCookies();
  } finally {
    await page.goto('about:blank');
    await server.close();
  }
});
