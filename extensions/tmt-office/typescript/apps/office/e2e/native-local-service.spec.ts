import { existsSync, readFileSync, writeFileSync, unlinkSync } from 'node:fs';
import { createConnection, createServer } from 'node:net';
import type { AddressInfo } from 'node:net';
import { once } from 'node:events';
import { get } from 'node:http';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import {
  installNativeOffice,
  unusedLoopbackPort,
  NATIVE_OFFICE_FIXTURE_VERSION,
} from './native-office-fixture.js';

test('missing Office installation fails without creating runtime or storage', async () => {
  await withSandbox(async (sandbox) => {
    const prefix = path.join(sandbox.root, 'missing-office');
    const start = await runCli(sandbox, ['office', 'start', '--prefix', prefix, '--json']);
    expect(start.status).toBe(1);
    expect(JSON.parse(start.stdout).error.code).toBe('OFFICE_NOT_INSTALLED');
    const stop = await runCli(sandbox, ['office', 'stop', '--prefix', prefix, '--json']);
    expect(stop.status, stop.stdout).toBe(0);
    expect(JSON.parse(stop.stdout)).toMatchObject({ running: false, changed: false });
    expect(existsSync(prefix)).toBe(false);
    expect(existsSync(sandbox.globalDir)).toBe(false);
  });
});

test('human Office start gives one private browser hint while reuse and JSON preserve their output contracts', async () => {
  await withSandbox(async (sandbox) => {
    delete sandbox.env.TMT_HINTS;
    const prefix = await installNativeOffice(sandbox);
    const office = (args: string[]) =>
      runCli(sandbox, ['office', '--prefix', prefix, ...args], { deadlineMs: 30_000 });
    const receiptPath = path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json');
    try {
      const first = await office(['start', '--port', String(await unusedLoopbackPort())]);
      expect(first.status, first.stdout + first.stderr).toBe(0);
      const urlText = first.stdout.trimEnd();
      expect(first.stdout).toBe(`${urlText}\n`);
      const url = new URL(urlText);
      expect(url.origin).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
      expect(url.pathname).toBe('/local');
      const token = new URLSearchParams(url.hash.slice(1)).get('token') ?? '';
      expect(token).toMatch(/^[A-Za-z0-9_-]{43}$/);
      expect(first.stderr).toBe(
        'Hint: Open the URL above in your local browser. Its private link belongs to the running service; use `tmt office start` to retrieve it later.\n'
      );
      expect(first.stderr).not.toContain(urlText);
      expect(first.stderr).not.toContain(token);

      const reused = await office(['start']);
      expect(reused.status, reused.stdout + reused.stderr).toBe(0);
      expect(reused.stdout).toBe(first.stdout);
      expect(reused.stderr).toBe('');

      const machine = await office(['start', '--json']);
      expect(machine.status, machine.stdout + machine.stderr).toBe(0);
      expect(machine.stderr).toBe('');
      expect(JSON.parse(machine.stdout)).toMatchObject({
        running: true,
        url: urlText,
        changed: false,
        reused: true,
        version: NATIVE_OFFICE_FIXTURE_VERSION,
      });
    } finally {
      const stopped = await office(['stop', '--json']);
      expect(stopped.status, stopped.stdout + stopped.stderr).toBe(0);
      expect(JSON.parse(stopped.stdout)).toMatchObject({ running: false });
      expect(existsSync(receiptPath)).toBe(false);
    }
  });
});

test('human Office start also mentions a pending storage migration once user Office data exists', async () => {
  await withSandbox(async (sandbox) => {
    delete sandbox.env.TMT_HINTS;
    const prefix = await installNativeOffice(sandbox);
    const office = (args: string[]) =>
      runCli(sandbox, ['office', '--prefix', prefix, ...args], { deadlineMs: 30_000 });
    const posted = await office([
      'board',
      'post',
      '--general',
      '--owner',
      '--title',
      'Hi',
      '--body',
      'Data',
      '--json',
    ]);
    expect(posted.status, posted.stdout + posted.stderr).toBe(0);
    try {
      const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
      expect(started.status, started.stdout + started.stderr).toBe(0);
      expect(started.stderr).toBe(
        'Hint: Open the URL above in your local browser. Its private link belongs to the running service; use `tmt office start` to retrieve it later.\n' +
          "Office storage migration available: run 'tmt office storage migrate' (the current storage keeps working until you do).\n"
      );
      const machine = await office(['start', '--json']);
      expect(machine.stderr).toBe('');
    } finally {
      const stopped = await office(['stop', '--json']);
      expect(stopped.status, stopped.stdout + stopped.stderr).toBe(0);
    }
  });
});

test('installed service reuses its session, isolates control authority and stops with an incomplete write', async () => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[]) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout).toBe(0);
      return JSON.parse(result.stdout);
    };
    const receiptPath = path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json');
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      expect(started).toMatchObject({ running: true, changed: true, reused: false });
      const url = new URL(started.url);
      const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
      const token = new URLSearchParams(url.hash.slice(1)).get('token');
      expect(token).toMatch(/^[A-Za-z0-9_-]{43}$/);
      expect(receipt.browserToken).toBe(token);
      expect(new Set([receipt.browserToken, receipt.controlToken, receipt.nonce]).size).toBe(3);
      for (const secret of [receipt.controlToken, receipt.nonce])
        expect(secret).toMatch(/^[A-Za-z0-9_-]{43}$/);
      const health = (bearer: string, nonce: string) =>
        fetch(`${url.origin}/control/v1/health`, {
          method: 'POST',
          headers: { Authorization: `Bearer ${bearer}`, 'X-TMT-Office-Nonce': nonce },
          signal: AbortSignal.timeout(10_000),
        });
      expect((await health(receipt.controlToken, receipt.nonce)).status).toBe(200);
      expect((await health(receipt.browserToken, receipt.nonce)).status).toBe(401);
      expect((await health(receipt.controlToken, receipt.browserToken)).status).toBe(401);
      const worldRead = (origin: string, bearer: string) =>
        fetch(`${origin}/api/v1/local/world`, {
          headers: { Authorization: `Bearer ${bearer}` },
          signal: AbortSignal.timeout(10_000),
        });
      expect((await worldRead(url.origin, receipt.controlToken)).status).toBe(401);
      const wrongHost = await new Promise<number | undefined>((resolve, reject) => {
        const request = get(
          `${url.origin}/api/v1/local/world`,
          {
            headers: { Host: 'attacker.invalid', Authorization: `Bearer ${token}` },
            signal: AbortSignal.timeout(10_000),
          },
          (response) => {
            response.resume();
            resolve(response.statusCode);
          }
        );
        request.on('error', reject);
      });
      expect(wrongHost).toBe(421);
      const shell = await fetch(`${url.origin}/local`, { signal: AbortSignal.timeout(10_000) });
      expect(shell.status).toBe(200);
      expect(shell.headers.get('cache-control')).toBe('no-store');
      expect(shell.headers.get('content-security-policy')).toBeTruthy();
      expect(shell.headers.get('access-control-allow-origin')).toBeNull();
      expect(await shell.text()).toContain('<div id="root"');
      expect(
        (await fetch(`${url.origin}/../private`, { signal: AbortSignal.timeout(10_000) })).status
      ).toBe(404);
      expect(await office(['start'])).toMatchObject({
        reused: true,
        changed: false,
        url: started.url,
      });
      const before = await office(['layout', 'show']);

      // A real socket holds an incomplete body, not a mocked handler or sleep.
      // The deadline bounds both worker shutdown and fixture cleanup on failure.
      const socket = createConnection({ host: '127.0.0.1', port: Number(url.port) });
      socket.setTimeout(10_000, () =>
        socket.destroy(new Error('Incomplete request did not close.'))
      );
      let response = '';
      socket.setEncoding('utf8');
      socket.on('data', (chunk) => {
        response += chunk;
        if (response.length > 4096)
          socket.destroy(new Error('Unexpected unbounded HTTP response.'));
      });
      const finished = once(socket, 'end');
      // An early socket error is rethrown by the awaited completion below.
      void finished.catch(() => {});
      try {
        await once(socket, 'connect');
        await new Promise<void>((resolve, reject) =>
          socket.write(
            `PUT /api/v1/local/world HTTP/1.1\r\nHost: ${url.host}\r\n` +
              `Authorization: Bearer ${token}\r\nOrigin: ${url.origin}\r\n` +
              'Content-Type: application/json\r\nContent-Length: 128\r\n\r\n{"expectedRevision":0,',
            (error) => (error ? reject(error) : resolve())
          )
        );
        expect((await office(['status'])).service.running).toBe(true);
        expect(await office(['stop'])).toMatchObject({ changed: true, running: false });
        await finished;
        expect(response).toMatch(/^HTTP\/1\.1 400 /);
      } finally {
        socket.destroy();
      }
      expect(existsSync(receiptPath)).toBe(false);
      expect(await office(['layout', 'show'])).toEqual(before);
      expect(await office(['stop'])).toMatchObject({ running: false, changed: false });

      const restarted = await office(['start', '--port', String(await unusedLoopbackPort())]);
      const nextReceipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
      for (const key of ['browserToken', 'controlToken', 'nonce'])
        expect(nextReceipt[key]).not.toBe(receipt[key]);
      const origin = new URL(restarted.url).origin;
      expect((await worldRead(origin, receipt.browserToken)).status).toBe(401);
      const fresh = await worldRead(origin, nextReceipt.browserToken);
      expect(fresh.status).toBe(200);
      expect(await fresh.json()).toEqual(before);
    } finally {
      expect(await office(['stop'])).toMatchObject({ running: false });
      expect(existsSync(receiptPath)).toBe(false);
    }
  });
});

test('service faults preserve authority boundaries and recover without orphan receipts', async () => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const office = async (args: string[], status = 0) => {
      const result = await runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
        deadlineMs: 30_000,
      });
      expect(result.status, result.stdout).toBe(status);
      return JSON.parse(result.stdout);
    };
    const receiptPath = path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json');
    await office(['start', '--port', String(await unusedLoopbackPort())]);
    try {
      const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
      try {
        writeFileSync(receiptPath, JSON.stringify({ ...receipt, runningVersion: '0.0.0-test' }));
        expect((await office(['status'])).service.restartNeeded).toBe(true);
        expect((await office(['start'], 1)).error.code).toBe('OFFICE_RESTART_REQUIRED');
      } finally {
        writeFileSync(receiptPath, JSON.stringify(receipt));
      }
      // The verified receipt belongs to this fixture's detached child, never a
      // host-discovered PID or the deliberately uncertain receipt tested below.
      expect(receipt.pid).not.toBe(process.pid);
      expect((await office(['status'])).service.running).toBe(true);
      process.kill(receipt.pid, 'SIGKILL');
      await expect
        .poll(() => {
          try {
            process.kill(receipt.pid, 0);
            return true;
          } catch (error) {
            if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
            return false;
          }
        })
        .toBe(false);
      expect((await office(['status'])).service.running).toBe(false);
      expect(await office(['start', '--port', String(await unusedLoopbackPort())])).toMatchObject({
        running: true,
        reused: false,
      });
      expect(await office(['stop'])).toMatchObject({ changed: true, running: false });
      expect(existsSync(receiptPath)).toBe(false);

      const occupied = createServer((socket) => socket.destroy());
      try {
        occupied.listen(0, '127.0.0.1');
        await once(occupied, 'listening');
        const port = (occupied.address() as AddressInfo).port;
        expect((await office(['start', '--port', String(port)], 1)).error.code).toBe(
          'OFFICE_PORT_UNAVAILABLE'
        );
        expect(existsSync(receiptPath)).toBe(false);
      } finally {
        await new Promise<void>((resolve, reject) =>
          occupied.close((error) => (error ? reject(error) : resolve()))
        );
      }

      // A plausible receipt for an unrelated live process must never authorize
      // killing that process or claim that its service is healthy.
      const uncertain = { ...receipt, pid: process.pid, port: 9, runningVersion: 'unrelated' };
      writeFileSync(receiptPath, JSON.stringify(uncertain), { mode: 0o600 });
      try {
        expect((await office(['status'], 1)).error.code).toBe('OFFICE_SERVICE_UNCERTAIN');
        expect(JSON.parse(readFileSync(receiptPath, 'utf8'))).toEqual(uncertain);
        expect(() => process.kill(process.pid, 0)).not.toThrow();
      } finally {
        unlinkSync(receiptPath);
      }
    } finally {
      expect(await office(['stop'])).toMatchObject({ running: false });
      expect(existsSync(receiptPath)).toBe(false);
    }
  });
});

test('a probe-valid companion that exits before readiness leaves no running service', async () => {
  await withSandbox(async (sandbox) => {
    const executable = path.join(sandbox.root, 'exits-before-readiness');
    writeFileSync(
      executable,
      '#!/bin/sh\nif [ "$1 $2 $3" = "__tmt-office 1 probe" ]; then\n' +
        `  printf 'TMT-OFFICE/1\\n${NATIVE_OFFICE_FIXTURE_VERSION}\\n'\n  exit 0\nfi\nexit 1\n`,
      { mode: 0o700 }
    );
    const prefix = await installNativeOffice(sandbox, executable);
    const result = await runCli(
      sandbox,
      [
        'office',
        '--prefix',
        prefix,
        'start',
        '--port',
        String(await unusedLoopbackPort()),
        '--json',
      ],
      { deadlineMs: 30_000 }
    );
    try {
      expect(result.status).toBe(1);
      expect(JSON.parse(result.stdout).error.code).toBe('OFFICE_SERVICE_UNAVAILABLE');
      expect(existsSync(path.join(sandbox.globalDir, 'office', 'runtime', 'service-v1.json'))).toBe(
        false
      );
    } finally {
      const stopped = await runCli(sandbox, ['office', '--prefix', prefix, 'stop', '--json']);
      expect(stopped.status, stopped.stdout).toBe(0);
      expect(JSON.parse(stopped.stdout)).toMatchObject({ running: false, changed: false });
    }
  });
});
