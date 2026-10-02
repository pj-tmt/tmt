import path from 'node:path';
import { expect, it } from 'vitest';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { createArtifact } from '../support/native-artifact.js';

it(
  'serves a maximum board page through 50 sequential installed-companion replies',
  { timeout: 240_000 },
  async () => {
    await withSandbox(async (sandbox) => {
      const prefix = path.join(sandbox.root, 'native install prefix with spaces');
      const fixture = await createArtifact(sandbox, '0.1.0-alpha.4', new Uint8Array(), 'office');
      const office = (args: string[], outputLimitBytes = 1024 * 1024) =>
        runCli(sandbox, ['office', '--prefix', prefix, ...args, '--json'], {
          deadlineMs: 30_000,
          outputLimitBytes,
        });
      const installed = await office([
        'install',
        '--yes',
        '--archive',
        fixture.archive,
        '--manifest',
        fixture.manifest,
      ]);
      expect(installed.status, installed.stdout + installed.stderr).toBe(0);
      const identity = await runCli(sandbox, ['identity', 'create', 'Alice', '--json']);
      expect(identity.status, identity.stdout + identity.stderr).toBe(0);
      const post = await office([
        'board',
        'post',
        '--general',
        '--identity',
        'Alice',
        '--title',
        'worst',
        '--body',
        '\t'.repeat(16_384),
      ]);
      expect(post.status, post.stdout + post.stderr).toBe(0);
      const threadId = (parseWholeStdout(post) as { threadId: string }).threadId;
      for (let index = 0; index < 50; index += 1) {
        const reply = await office([
          'board',
          'reply',
          threadId,
          '--identity',
          'Alice',
          '--body',
          '\t'.repeat(8_192),
        ]);
        expect(reply.status, `reply ${index}: ${reply.stdout}${reply.stderr}`).toBe(0);
      }
      const page = await office(
        ['board', 'show', threadId, '--reply-limit', '50'],
        2 * 1024 * 1024
      );
      expect(page.status, page.stdout + page.stderr).toBe(0);
      const document = parseWholeStdout(page) as {
        thread: { body: string };
        replies: { body: string }[];
      };
      expect(document.thread.body).toHaveLength(16_384);
      expect(document.replies).toHaveLength(50);
      expect(document.replies.every((reply) => reply.body.length === 8_192)).toBe(true);
    });
  }
);
