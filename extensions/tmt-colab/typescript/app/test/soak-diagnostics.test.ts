import { describe, expect, it } from 'vite-plus/test';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  realpathSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { SoakCollector } from '../acceptance/harness/soak-collector.js';
import SoakReporter, { apiSpan } from '../acceptance/harness/soak-reporter.js';
import { appendSoakInvocation } from '../acceptance/harness/soak-diagnostics.js';
import {
  policy,
  loadSoakDictionary,
  type SoakDictionary,
} from '../acceptance/harness/soak-literals.mjs';

const secret = 'PRIVATE_CREDENTIAL_SENTINEL';

describe('synthetic soak API projection', () => {
  const span = { category: 'pw:api', title: 'Click', startTime: new Date(1000), duration: 40 };
  it('accepts only exact API category/name and never a test-authored step', () => {
    expect(apiSpan(span, 900, 160)).toEqual({
      name: 'locator.click',
      startMs: 100,
      durationMs: 40,
    });
    expect(apiSpan({ ...span, category: 'test.step' }, 900, 160)).toBeUndefined();
    expect(apiSpan({ ...span, title: 'Click ' + secret }, 900, 160)).toBeUndefined();
    expect(apiSpan({ ...span, title: secret }, 900, 160)).toBeUndefined();
    expect(apiSpan({ ...span, duration: -1 }, 900, 160)).toEqual({
      name: 'locator.click',
      startMs: 100,
      durationMs: 60,
    });
  });

  it('does not load without an attachment and drops evidence when reporter loading fails', () => {
    let loads = 0;
    const reporter = new SoakReporter({
      loadDictionary: () => {
        loads++;
        throw new Error('Fixture compiler unavailable');
      },
    });
    const test = {
      title: policy.scenario,
      location: { file: '/fixture/connection-soak.spec.ts' },
    } as Parameters<SoakReporter['onTestEnd']>[0];
    const result = {
      status: 'failed',
      startTime: new Date(0),
      attachments: [],
    } as unknown as Parameters<SoakReporter['onTestEnd']>[1];
    expect(() => reporter.onTestEnd(test, result)).not.toThrow();
    expect(loads).toBe(0);
    result.attachments.push({
      name: policy.attachment,
      contentType: 'application/json',
      path: secret,
      body: Buffer.from(JSON.stringify({ originMs: 0, observedMs: 1 })),
    });
    expect(() => reporter.onTestEnd(test, result)).not.toThrow();
    expect(loads).toBe(1);
    expect(result.attachments[0].body).toBeUndefined();
    expect(result.attachments[0].path).toBeUndefined();
  });

  it('strips private reporter metadata and keeps only the last 30 seconds of API spans', () => {
    const reporter = new SoakReporter({});
    const test = {
      title: policy.scenario,
      location: { file: '/fixture/connection-soak.spec.ts' },
    } as Parameters<SoakReporter['onTestEnd']>[0];
    const result = {
      status: 'failed',
      startTime: new Date(0),
      attachments: [
        {
          name: policy.attachment,
          contentType: 'application/json',
          path: secret,
          body: Buffer.from(
            JSON.stringify({
              version: 1,
              scenario: policy.id,
              observedMs: 31000,
              originMs: 1000,
              assertion: 'directory',
              truncated: { spans: true },
              error: { message: secret },
            }),
          ),
        },
      ],
    } as Parameters<SoakReporter['onTestEnd']>[1];
    for (const category of ['test.step', 'pw:api'])
      reporter.onStepBegin(test, result, {
        ...span,
        category,
        startTime: new Date(2000),
        duration: 100,
        params: { secret },
        subtitle: secret,
      } as unknown as Parameters<SoakReporter['onStepBegin']>[2]);
    reporter.onStepBegin(test, result, {
      ...span,
      startTime: new Date(1000),
      duration: 10,
    } as unknown as Parameters<SoakReporter['onStepBegin']>[2]);
    reporter.onTestEnd(test, result);
    const attachment = result.attachments[0];
    expect(attachment.path).toBeUndefined();
    const retained = attachment.body!.toString();
    expect(retained).not.toContain(secret);
    expect(retained).not.toContain('originMs');
    expect(JSON.parse(retained).truncated.spans).toBe(true);
    expect(JSON.parse(retained).spans).toEqual([
      { name: 'locator.click', startMs: 1000, durationMs: 100 },
    ]);
  });
});

describe('soak failure collection before world cleanup', () => {
  it('loads once on first need, shares the result and disables capture on dictionary failure', async () => {
    let loads = 0;
    let read: (() => unknown) | undefined;
    let observe: (() => Promise<void>) | undefined;
    let attachments = 0;
    const world = {
      enableSoakDiagnostics(dictionary: () => unknown) {
        read = dictionary;
      },
      onFailure(callback: () => Promise<void>) {
        observe = callback;
      },
    } as unknown as ConstructorParameters<typeof SoakCollector>[0];
    const info = {
      attach: async () => {
        attachments++;
      },
    } as unknown as ConstructorParameters<typeof SoakCollector>[1];
    expect(
      () =>
        new SoakCollector(world, info, () => {
          loads++;
          throw new Error('Fixture compiler unavailable');
        }),
    ).not.toThrow();
    expect(loads).toBe(0);
    expect(typeof read).toBe('function');
    expect(() => read!()).not.toThrow();
    expect(read!()).toBeUndefined();
    expect(loads).toBe(1);
    await expect(observe!()).resolves.toBeUndefined();
    expect(loads).toBe(1);
    expect(attachments).toBe(0);
  });

  it.each([false, true])(
    'captures setup failure and owned listener liveness=%s without a browser',
    async (listening) => {
      const root = realpathSync(mkdtempSync(path.join(realpathSync('/tmp'), 'tmt-acc-collector-')));
      let observe: (() => Promise<void>) | undefined;
      let body: Buffer | undefined;
      let dictionary: (() => SoakDictionary | undefined) | undefined;
      let loads = 0;
      const servers: net.Server[] = [];
      const world = {
        root,
        dataRoot: path.join(root, 'data'),
        enableSoakDiagnostics(read: () => SoakDictionary | undefined) {
          dictionary = read;
        },
        onFailure(callback: () => Promise<void>) {
          observe = callback;
        },
        soakLogs: () => ({ records: [], truncated: false }),
        soakChildren: () => [],
      } as unknown as ConstructorParameters<typeof SoakCollector>[0];
      const info = {
        attach: async (_name: string, attachment: { body: Buffer }) => {
          body = attachment.body;
        },
      } as unknown as ConstructorParameters<typeof SoakCollector>[1];
      try {
        const collector = new SoakCollector(world, info, () => {
          loads++;
          return loadSoakDictionary();
        });
        expect(loads).toBe(0);
        expect(dictionary!()).toBeDefined();
        expect(dictionary!()).toBeDefined();
        expect(loads).toBe(1);
        if (listening) {
          const directory = path.join(root, 'data', 'colab');
          mkdirSync(directory, { recursive: true });
          for (const address of [
            path.join(directory, 'door.sock'),
            { host: '127.0.0.1', port: 0 },
          ]) {
            const server = net.createServer((socket) => socket.destroy());
            servers.push(server);
            await new Promise<void>((resolve, reject) => {
              server.once('error', reject);
              if (typeof address === 'string') server.listen(address, resolve);
              else server.listen(address, resolve);
            });
          }
          const address = servers[1].address() as net.AddressInfo;
          collector.setDoor({
            origin: `http://127.0.0.1:${address.port}`,
            remote: { child: { exitCode: null } },
          });
        }
        appendSoakInvocation(
          root,
          'core',
          3,
          null,
          JSON.stringify({ error: { code: 'PANE_NOT_FOUND', message: secret }, payload: secret }),
        );
        await observe!();
        expect(loads).toBe(1);
        const retained = JSON.parse(body!.toString());
        expect(retained.assertion.id).toBe('setup');
        expect(retained.logs.some((row: { code: string }) => row.code === 'PANE_NOT_FOUND')).toBe(
          true,
        );
        expect(retained.children).toEqual([
          { index: 0, role: 'core', alive: false, exitCode: 3, signal: null },
        ]);
        expect(retained.alive).toEqual({
          socket: listening,
          door: listening,
          method: 'owned-resource-probe',
        });
        expect(body!.toString()).not.toContain(secret);
        expect(body!.toString()).not.toContain(root);
      } finally {
        await Promise.all(
          servers
            .filter((server) => server.listening)
            .map((server) => new Promise<void>((resolve) => server.close(() => resolve()))),
        );
        expect(servers.every((server) => !server.listening)).toBe(true);
        rmSync(root, { recursive: true, force: true });
      }
    },
  );

  it('does not append through a native-log symlink or to an unrelated root', () => {
    const root = realpathSync(mkdtempSync(path.join(realpathSync('/tmp'), 'tmt-acc-collector-')));
    const outside = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'soak-native-outside-')));
    try {
      const file = path.join(outside, 'private');
      writeFileSync(file, secret);
      symlinkSync(file, path.join(root, 'soak-native.stderr'));
      appendSoakInvocation(root, 'core', 1, null, '{}');
      appendSoakInvocation(outside, 'core', 1, null, '{}');
      expect(readFileSync(file, 'utf8')).toBe(secret);
      expect(existsSync(path.join(outside, 'soak-native.stderr'))).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
      rmSync(outside, { recursive: true, force: true });
    }
  });
});
