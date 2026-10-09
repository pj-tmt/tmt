// Test evidence only: no progress marker grants admission or substitutes for the report gate.
export const PROGRESS_PREFIX = 'colab-conformance:';
export function engineDiagnostics() {
  const started = performance.now();
  const evidence = { lifecycle: [], progress: {}, cleanup: 'not-started' };
  const event = (name) => {
    if (evidence.lifecycle.length < 32)
      evidence.lifecycle.push({ event: name, elapsedMs: Math.round(performance.now() - started) });
  };
  const failure = (error) => {
    evidence.errorStack = error instanceof Error ? error.stack : String(error);
  };
  return {
    evidence,
    event,
    failure,
    browser(browser) {
      event('browser-launched');
      browser.on('disconnected', () => event('browser-disconnected'));
    },
    page(page) {
      event('page-created');
      page.on('crash', () => event('page-crash'));
      page.on('close', () => event('page-close'));
      page.context().on('close', () => event('context-close'));
      page.on('console', (message) => {
        const text = message.text();
        if (!text.startsWith(PROGRESS_PREFIX) || text.length > 512) return;
        try {
          const [state, id] = JSON.parse(text.slice(PROGRESS_PREFIX.length));
          if (
            ['started', 'completed'].includes(state) &&
            typeof id === 'string' &&
            id.length <= 200
          )
            evidence.progress[state] = id;
        } catch {
          // Unrelated console output cannot affect conformance or diagnostics.
        }
      });
    },
    async close(browser, timeoutMs = 30000) {
      evidence.cleanup = 'started';
      event('cleanup-started');
      let timer;
      try {
        await Promise.race([
          browser.close(),
          new Promise((_, reject) => {
            timer = setTimeout(
              () => reject(new Error('Browser cleanup deadline exceeded')),
              timeoutMs,
            );
          }),
        ]);
        evidence.cleanup = 'closed';
        event('cleanup-closed');
      } catch (error) {
        evidence.cleanup = 'failed';
        evidence.cleanupError = error instanceof Error ? error.stack : String(error);
        throw error;
      } finally {
        clearTimeout(timer);
      }
    },
  };
}
