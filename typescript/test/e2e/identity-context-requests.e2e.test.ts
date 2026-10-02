import Database from 'better-sqlite3';
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { requestAttempts, requestResponses } from './request-state-oracle.js';

function attentionState(fixture: E2EFixture): unknown {
  const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return {
      originators: db
        .prepare('SELECT * FROM request_attention_identities ORDER BY identity_id')
        .all(),
      recipients: db
        .prepare('SELECT * FROM request_recipient_attention_identities ORDER BY identity_id')
        .all(),
    };
  } finally {
    db.close();
  }
}

describe.sequential('identity context request summaries', () => {
  it('counts unacknowledged X items without consuming attention or exposing message content', async () => {
    await withE2EFixture(
      async (fixture) => {
        const reader = expectJsonResult<{ id: string }>(
          await fixture.runJsonCli(['name', 'Reader', '-s'])
        );
        expectJsonResult(
          await fixture.runJsonCli(['identity', 'create', 'Peer'], { withoutTmux: true })
        );
        const queue = async (sender: string, recipient: string, body: string) =>
          expectJsonResult<{ requestId: string }>(
            await fixture.runJsonCli(
              ['talk', recipient, body, '--inbox', '--identity', sender, '--detach'],
              { withoutTmux: true }
            )
          ).requestId;
        const pending = await queue('Reader', 'Peer', 'private pending body');
        expectJsonResult(
          await fixture.runJsonCli(['x', 'ackall', '--identity', 'Reader'], { withoutTmux: true })
        );
        const completed = await queue('Reader', 'Peer', 'private completed body');
        const detail = expectJsonResult<{ exchange: { reply: { receipt: string } } }>(
          await fixture.runJsonCli(['x', 'show', completed, '--incoming', '--identity', 'Peer'], {
            withoutTmux: true,
          })
        );
        expectJsonResult(
          await fixture.runJsonCli(
            [
              'reply',
              completed,
              '--receipt',
              detail.exchange.reply.receipt,
              '--message',
              'private final body',
            ],
            { withoutTmux: true }
          )
        );
        const incoming = await queue('Peer', 'Reader', 'private incoming body');
        const before = {
          identities: durableState(fixture),
          attempts: requestAttempts(fixture),
          responses: requestResponses(fixture),
          attention: attentionState(fixture),
        };
        const result = await fixture.runJsonCli(['whoami', '--context']);
        expectJsonResult(result);
        expect(result.json).toMatchObject({
          bound: true,
          id: reader.id,
          originated: { count: 1, inspect: `tmt x --identity '${reader.id}' --json` },
          incoming: { count: 1, inspect: `tmt x --incoming --identity '${reader.id}' --json` },
        });
        expect(result.stdout).not.toContain('private ');
        expect(result.stdout).not.toContain(detail.exchange.reply.receipt);
        for (const id of [pending, completed, incoming]) expect(result.stdout).not.toContain(id);
        expect(Buffer.byteLength(result.stdout)).toBeLessThanOrEqual(4096);
        expect({
          identities: durableState(fixture),
          attempts: requestAttempts(fixture),
          responses: requestResponses(fixture),
          attention: attentionState(fixture),
        }).toEqual(before);
      },
      { mode: 'input-log' }
    );
  });
});

it.each(['claude', 'codex'] as const)(
  '%s prompt context offers nonzero incoming attention without consuming it or reviving a session',
  async (provider) => {
    let roots: string[] = [];
    await withE2EFixture(
      async (fixture) => {
        roots = [fixture.root, fixture.socketRoot];
        // Publish the private server's TMT metadata before probing an unbound runtime pane.
        expectJsonResult(await fixture.runJsonCli(['name', 'Fixture Owner', '-s']));
        expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Sender']));
        const session = '11111111-1111-4111-8111-111111111111';
        const hook = (event: string, id = session) => ({
          args: ['__hook', provider],
          input: {
            hook_event_name: event,
            session_id: id,
            source: 'startup',
            reason: 'other',
            turn_id: 'same-turn',
          },
        });
        const beforePrompt = path.join(fixture.root, 'before-prompt');
        const afterPrompt = path.join(fixture.root, 'after-prompt');
        const scenario = path.join(fixture.root, 'inbox-prompt-scenario.json');
        const report = path.join(fixture.root, 'inbox-prompt-report.json');
        const steps = [
          hook('UserPromptSubmit'), // unbound
          { args: ['name', 'Prompt Inbox', '-s', '--json'] },
          hook('UserPromptSubmit'), // bound but unadmitted
          hook('SessionStart'),
          hook('UserPromptSubmit'), // admitted, zero attention and zero extensions
          {
            args: [
              'talk',
              'Prompt Inbox',
              'private queued body',
              '--inbox',
              '--identity',
              'Sender',
              '--detach',
              '--json',
            ],
          },
          hook('UserPromptSubmit', '22222222-2222-4222-8222-222222222222'),
          { ...hook('UserPromptSubmit'), checkpoint: beforePrompt },
          { ...hook('UserPromptSubmit'), checkpoint: afterPrompt },
          { args: ['x', 'ackall', '--incoming', '--identity', 'Prompt Inbox', '--json'] },
          hook('UserPromptSubmit'), // attention acknowledged, request remains open
          hook('SessionEnd'),
          hook('UserPromptSubmit'), // ended must not revive
        ];
        fs.writeFileSync(scenario, JSON.stringify(steps));
        const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
        const command = [
          'env',
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          provider === 'claude' ? '/opt/tmt-tests/claude' : '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          scenario,
          report,
        ]
          .map(quote)
          .join(' ');
        const pane = fixture.createShellPane('inbox-prompt-context').pane;
        fixture.tmux(['send-keys', '-t', pane, '-l', command]);
        fixture.tmux(['send-keys', '-t', pane, 'Enter']);
        const state = () => ({
          identities: durableState(fixture),
          attempts: requestAttempts(fixture),
          responses: requestResponses(fixture),
          attention: attentionState(fixture),
        });
        await fixture.waitFor(() => fs.existsSync(beforePrompt), 15000, 'before verified prompt');
        const before = state();
        const pending = before.attempts.find((row) => row.message_text === 'private queued body')!;
        expect(pending).toMatchObject({ status: 'queued', wake_state: 'not_attempted' });
        fs.writeFileSync(beforePrompt, 'continue');
        await fixture.waitFor(() => fs.existsSync(afterPrompt), 5000, 'after verified prompt');
        // Observe independently: no service read, retention cleanup or acknowledgement.
        expect(state()).toEqual(before);
        fs.writeFileSync(afterPrompt, 'continue');
        await fixture.waitFor(() => fs.existsSync(report), 15000, 'prompt inbox report');
        const results = JSON.parse(fs.readFileSync(report, 'utf8')) as Array<{
          code: number;
          stdout: string;
          stderr: string;
        }>;
        expect(results).toHaveLength(steps.length);
        for (const [index, item] of results.entries()) {
          expect(item.code, `step ${index}: ${item.stderr}`).toBe(0);
          expect(item.stderr, `step ${index}`).toBe('');
        }
        for (const index of [0, 2, 4, 6, 10, 11, 12]) expect(results[index].stdout).toBe('');
        const id = JSON.parse(results[1].stdout).id;
        const line = `Incoming X items: 1 unacknowledged; pull with tmt inbox --identity '${id}' --json\n`;
        const context = JSON.parse(results[7].stdout);
        expect(context).toEqual({
          hookSpecificOutput: { hookEventName: 'UserPromptSubmit', additionalContext: line },
        });
        expect(results[8].stdout).toBe(results[7].stdout);
        expect(line.trimEnd().split('\n')).toHaveLength(1);
        expect(results[7].stdout).not.toContain('private queued body');
        expect(results[7].stdout).not.toContain(pending.request_id);
        expect(results[7].stdout).not.toContain('receipt');
        expect(results[7].stdout).not.toContain('TMT identity:');
        // Attention is distinct from unanswered/unsent: ack removes the hint, not the request.
        const open = expectJsonResult<{ items: Array<{ requestId: string }> }>(
          await fixture.runJsonCli(['inbox', '--identity', id], { withoutTmux: true })
        );
        expect(open.items.map((item) => item.requestId)).toContain(pending.request_id);
        expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
      },
      { mode: 'input-log' }
    );
    expect(roots.every((root) => !fs.existsSync(root))).toBe(true);
  }
);
