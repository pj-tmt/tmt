import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { chmod, copyFile, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import type { Page } from '@playwright/test';

/**
 * A private door for the one-TMT-app conditions: the real `tmt-remote serve` and the real
 * `tmt-colab serve` in a throwaway HOME, with Remote's public-core fixture as the only core. It
 * never reads the owner's TMT data, serve, session or config, and starts no tmux. The processes
 * are the ones this object spawned and `stop` ends exactly those.
 */
const binary = (variable: string, name: string) =>
  process.env[variable] ??
  fileURLToPath(new URL(`../../../../../rust/target/debug/${name}`, import.meta.url));
export const remoteBinary = binary('TMT_REMOTE_BINARY', 'tmt-remote');
export const colabBinary = binary('TMT_COLAB_BINARY', 'tmt-colab');
const coreFixture = fileURLToPath(
  new URL('../../../../tmt-remote/typescript/remote-client/e2e/core-fixture.mjs', import.meta.url),
);

export const binariesBuilt = () => existsSync(remoteBinary) && existsSync(colabBinary);

export interface IsolatedDoor {
  /** `http://127.0.0.1:<port>` */
  origin: string;
  /** The machine's route prefix, `<origin>/r/<prefix>`. */
  address: string;
  /** Colab's mount, `<address>/x/colab/`. */
  mount: string;
  env: NodeJS.ProcessEnv;
  /** Creates a page in the door's private space through the real CLI; returns its id. */
  createPage(title: string, html: string): Promise<string>;
  /** Pairs the page's browser context with the door, as a person does with `tmt remote pair`. */
  pair(page: Page, name?: string): Promise<void>;
  /** Stops and starts the Remote serve again at the same origin: its sessions end, pairings stay. */
  restartRemote(): Promise<void>;
  stop(): Promise<void>;
}

function firstLine(child: ChildProcessWithoutNullStreams): Promise<string> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('No descriptor within 30 s.')), 30000);
    createInterface({ input: child.stdout }).once('line', (line) => {
      clearTimeout(timer);
      resolve(line);
    });
    child.once('exit', (code) => reject(new Error(`Exited with ${code} before its descriptor.`)));
  });
}
const exited = (child: ChildProcessWithoutNullStreams) =>
  child.exitCode !== null || child.signalCode !== null
    ? Promise.resolve()
    : new Promise((resolve) => child.once('exit', resolve));

export async function startIsolatedDoor(): Promise<IsolatedDoor> {
  // Short root: Unix socket paths are limited to about 100 bytes.
  const root = await mkdtemp('/tmp/tmt-spa-door-');
  const children: ChildProcessWithoutNullStreams[] = [];
  const stop = async () => {
    for (const child of children.reverse()) {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    }
    await Promise.all(children.map(exited));
    await rm(root, { recursive: true, force: true });
  };
  try {
    const core = `${root}/core.mjs`;
    await copyFile(coreFixture, core);
    await chmod(core, 0o700);
    // An explicit environment: no TMUX, no inherited TMT state, only this HOME.
    const env: NodeJS.ProcessEnv = {
      PATH: process.env.PATH,
      HOME: root,
      TMT_AGENT: '1',
      BROWSER: 'true',
      TMT_EXECUTABLE: core,
      TMT_FIXTURE_ROOT: root,
    };
    const serveRemote = async () => {
      const child = spawn(remoteBinary, ['serve', '--json'], { env, cwd: root });
      children.push(child);
      const { address } = JSON.parse(await firstLine(child)) as { address: string };
      return { child, address };
    };
    let { child: remote, address } = await serveRemote();
    const colab = spawn(colabBinary, ['serve', '--json'], { env, cwd: root });
    children.push(colab);
    await firstLine(colab);
    return {
      origin: new URL(address).origin,
      address,
      mount: `${address}/x/colab/`,
      env,
      async createPage(title, html) {
        const file = `${root}/${title.replaceAll(/\W+/g, '-')}.html`;
        await writeFile(file, html);
        const created = spawn(
          colabBinary,
          ['page', 'create', '--title', title, '--layout', 'column', '--file', file, '--json'],
          { env, cwd: root },
        );
        children.push(created);
        let out = '';
        created.stdout.on('data', (chunk: Buffer) => (out += chunk));
        let err = '';
        created.stderr.on('data', (chunk: Buffer) => (err += chunk));
        await exited(created);
        if (created.exitCode !== 0) throw new Error(`page create failed: ${err || out}`);
        const result = JSON.parse(out) as { id?: string; pageId?: string };
        return (result.id ?? result.pageId)!;
      },
      async pair(page, name = 'Isolated browser') {
        const pairing = spawn(remoteBinary, ['pair', '--json'], { env, cwd: root });
        children.push(pairing);
        const lines = createInterface({ input: pairing.stdout })[Symbol.asyncIterator]();
        const next = async () => JSON.parse((await lines.next()).value as string);
        const offer = await next();
        await page.goto(offer.link as string);
        await page.locator('#name').fill(name, { timeout: 10000 });
        await page.click('button[type="submit"]');
        await page.locator('#words').waitFor();
        await next(); // the candidate whose words the page shows
        pairing.stdin.write('confirm\n');
        await next(); // paired
        await page.locator('#mark').filter({ hasText: '✓' }).waitFor();
        await exited(pairing);
      },
      async restartRemote() {
        remote.kill('SIGTERM');
        await exited(remote);
        const next = await serveRemote();
        if (next.address !== address)
          throw new Error(`Remote came back at ${next.address}, not ${address}.`);
        remote = next.child;
      },
      stop,
    };
  } catch (error) {
    await stop();
    throw error;
  }
}
