import { spawnSync } from 'node:child_process';
import { describe, expect, it } from 'vite-plus/test';

// Drive the same event interface as Playwright without requiring installed browser binaries
// in the model unit gate. The separately labelled Linux run owns actual engine evidence.
const setup = `
import { strict as assert } from 'node:assert';
import { EventEmitter } from 'node:events';
import { engineDiagnostics, PROGRESS_PREFIX } from ${JSON.stringify(new URL('./diagnostics.mjs', import.meta.url).href)};
const browser = new EventEmitter(), context = new EventEmitter(), page = new EventEmitter();
page.context = () => context;
browser.close = async () => { page.emit('close'); context.emit('close'); browser.emit('disconnected'); };
const d = engineDiagnostics(); d.browser(browser); d.page(page);
const progress = (state, id) => page.emit('console', { text: () => PROGRESS_PREFIX + JSON.stringify([state, id]) });
`;
function run(script: string) {
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', setup + script], {
    timeout: 5000,
    encoding: 'utf8',
  });
  expect(result.error, result.stderr).toBeUndefined();
  expect(result.status, result.stderr).toBe(0);
}
describe('engine failure evidence', () => {
  it('retains completed/current checks, crash order and stack through cleanup after target closure', () => {
    run(`
      progress('completed', 'capabilities');
      progress('started', 'ed25519:positive-normal');
      page.emit('crash');
      const failure = new Error('page.evaluate: Target page, context or browser has been closed');
      d.failure(failure);
      await d.close(browser);
      const evidence = JSON.parse(JSON.stringify(d.evidence));
      assert.deepEqual(evidence.progress, { completed: 'capabilities', started: 'ed25519:positive-normal' });
      assert.equal(evidence.errorStack, failure.stack);
      assert.equal(evidence.cleanup, 'closed');
      assert.deepEqual(evidence.lifecycle.map(e => e.event), [
        'browser-launched', 'page-created', 'page-crash', 'cleanup-started',
        'page-close', 'context-close', 'browser-disconnected', 'cleanup-closed'
      ]);
      assert(evidence.lifecycle.every((e, i, rows) => e.elapsedMs >= 0 && (!i || e.elapsedMs >= rows[i - 1].elapsedMs)));
    `);
  });
  it('retains successful completion and ignores unrelated or malformed console output', () => {
    run(`
      progress('completed', 'signin-and-management');
      for (const text of ['ordinary console', PROGRESS_PREFIX + 'not-json', PROGRESS_PREFIX + '{}',
        PROGRESS_PREFIX + JSON.stringify(['completed', 'x'.repeat(201)]),
        PROGRESS_PREFIX + JSON.stringify(['unknown', 'capabilities'])])
        page.emit('console', { text: () => text });
      await d.close(browser);
      assert.deepEqual(d.evidence.progress, { completed: 'signin-and-management' });
      assert.equal(d.evidence.errorStack, undefined);
      assert.equal(d.evidence.cleanup, 'closed');
    `);
  });
  it('records cleanup rejection without discarding the original evaluation failure', () => {
    run(`
      const failure = new Error('original target closure');
      d.failure(failure);
      browser.close = async () => { throw new Error('close failed'); };
      await assert.rejects(d.close(browser), /close failed/);
      assert.equal(d.evidence.cleanup, 'failed');
      assert.match(d.evidence.cleanupError, /close failed/);
      assert.equal(d.evidence.errorStack, failure.stack);
    `);
  });
  it('bounds an unresponsive close and leaves it a failure', () => {
    run(`
      browser.close = () => new Promise(() => {});
      await assert.rejects(d.close(browser, 20), /Browser cleanup deadline exceeded/);
      assert.equal(d.evidence.cleanup, 'failed');
      assert.match(d.evidence.cleanupError, /deadline exceeded/);
      assert(!d.evidence.lifecycle.some(e => e.event === 'cleanup-closed'));
    `);
  });
});
