import { AcceptanceWorld } from './world.js';

const active = new Set<AcceptanceWorld>();

/**
 * Run a scenario in a fresh world. The callback's failure is preserved, but
 * cleanup and the leak report always run; a leak fails an otherwise green run.
 */
export async function withWorld<T>(scenario: (world: AcceptanceWorld) => Promise<T>): Promise<T> {
  const world = new AcceptanceWorld();
  active.add(world);
  let result: T | undefined;
  let failure: unknown;
  let failed = false;
  try {
    await world.start();
    result = await scenario(world);
  } catch (error) {
    failed = true;
    failure = error;
  }
  const leaks = await world.dispose();
  active.delete(world);
  if (failed) throw failure;
  if (leaks.length > 0) throw new Error(`Acceptance world leaked:\n${leaks.join('\n')}`);
  return result as T;
}

/**
 * Register as `test.afterEach`: a test that times out never reaches the
 * cleanup in `withWorld`, so its servers, tmux server and browsers are stopped here, and what
 * they leave is reported: a timed-out test must not leak silently.
 */
export async function disposeActiveWorlds(): Promise<void> {
  const leaks: string[] = [];
  for (const world of active) {
    active.delete(world);
    for (const leak of await world.dispose()) leaks.push(`${world.root}: ${leak}`);
  }
  if (leaks.length > 0) throw new Error(`Acceptance world leaked:\n${leaks.join('\n')}`);
}
