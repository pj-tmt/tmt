import { existsSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox } from '../support/cli-process.js';
import { resolveCliExecutables } from '../support/cli-executable.mjs';

// The ordinary sandbox runner owns neutral ancestry, the process group and cleanup.
// This scenario owns only terminal input and a local, non-forwarding HTTP proxy.
// No test-specific endpoint, certificate or production admission bypass is used.
const terminal = String.raw`
import errno, fcntl, json, os, pty, select, signal, socket, struct, subprocess, sys, termios, time
executable, answer = sys.argv[1:3]
proxy = socket.socket()
proxy.bind(('127.0.0.1', 0))
proxy.listen(4)
proxy.setblocking(False)
environment = dict(os.environ)
environment['HTTPS_PROXY'] = 'http://127.0.0.1:' + str(proxy.getsockname()[1])
environment['HTTP_PROXY'] = environment['HTTPS_PROXY']
environment['ALL_PROXY'] = environment['HTTPS_PROXY']
environment['NO_PROXY'] = ''
environment['NO_COLOR'] = '1'
environment['TERM'] = 'xterm'
if answer != 'agent': environment.pop('TMT_AGENT', None)
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 96, 0, 0))
child = subprocess.Popen([executable, 'ops', '--help'] if answer == 'help' else [executable, 'ops', 'sq', 'ls'], stdin=slave, stdout=slave, stderr=slave, env=environment)
os.close(slave)
screen = b''
sent = False
connections = 0
before_answer = 0
deadline = time.monotonic() + 10
try:
    while True:
        if time.monotonic() >= deadline: raise RuntimeError('terminal scenario exceeded its bound')
        ready, _, _ = select.select([master, proxy], [], [], max(0, deadline - time.monotonic()))
        if proxy in ready:
            connection, _ = proxy.accept()
            connections += 1
            connection.close() # Never forward to the Internet.
        if master in ready:
            try: chunk = os.read(master, 65536)
            except OSError as error:
                if error.errno == errno.EIO: break
                raise
            if not chunk: break
            screen += chunk
            if len(screen) > 1024 * 1024: raise RuntimeError('terminal output exceeded its bound')
            if not sent and b'Install it now?' in screen:
                sent = True
                before_answer = connections
                if answer == 'interrupt': child.send_signal(signal.SIGINT)
                else: os.write(master, {'no':b'n\n', 'eof':b'\x04', 'yes':b'yes\n', 'enter':b'\n'}[answer])
    status = child.wait(timeout=2)
    print(json.dumps({'status':status, 'asked':sent, 'connections':connections, 'beforeAnswer':before_answer, 'screen':screen.decode('utf-8')}))
finally:
    if child.poll() is None: child.kill()
    child.wait(timeout=2)
    os.close(master)
    proxy.close()
`;

describe('a missing official extension in a private human terminal', () => {
  it.each(['no', 'eof', 'interrupt', 'yes', 'enter', 'agent', 'help'] as const)(
    '%s follows consent without executing the original command or leaving a partial install',
    async (answer) => {
      const { cli } = resolveCliExecutables();
      await withSandbox(
        async (sandbox) => {
          const result = await runCli(sandbox, [], { deadlineMs: 15_000 });
          expect(result.status, result.stderr + result.stdout).toBe(0);
          const observation = JSON.parse(result.stdout) as {
            status: number;
            asked: boolean;
            connections: number;
            beforeAnswer: number;
            screen: string;
          };
          if (answer === 'agent' || answer === 'help') {
            expect(observation.asked).toBe(false);
            expect(observation.connections).toBe(0);
            expect(observation.status).toBe(3);
            expect(observation.screen).toContain('tmt extension install ops --yes');
          } else {
            expect(observation.asked, observation.screen).toBe(true);
            // Failed preview still permits the question with unknown metadata.
            expect(observation.screen).toContain('version unknown');
            expect(observation.screen).toContain('size unknown');
            if (answer === 'yes' || answer === 'enter') {
              expect(observation.status).toBe(1);
              expect(observation.connections).toBeGreaterThan(observation.beforeAnswer);
              expect(observation.screen).toContain('Could not check for ops updates');
            } else {
              expect(observation.status).toBe(answer === 'interrupt' ? -2 : 0);
              expect(observation.beforeAnswer).toBeGreaterThan(0);
              expect(observation.connections).toBe(observation.beforeAnswer);
              if (answer !== 'interrupt') expect(observation.screen).toContain('No changes made');
            }
          }
          expect(existsSync(path.join(sandbox.home, '.local'))).toBe(false);
          expect(existsSync(path.join(sandbox.home, '.agents'))).toBe(false);
          expect(existsSync(sandbox.database)).toBe(false);
          expect(observation.screen).not.toContain('Run it again:');
        },
        {
          TMT_TEST_CLI: JSON.stringify({
            executable: '/usr/bin/python3',
            args: ['-c', terminal, cli.executable, answer],
          }),
        }
      );
    },
    20_000
  );
});
