import { L5World } from './world.js';

/**
 * Run a scenario in a fresh world. The callback's failure is preserved, but
 * cleanup and the leak report always run; a leak fails an otherwise green run.
 */
export async function withWorld<T>(scenario: (world: L5World) => Promise<T>): Promise<T> {
  const world = new L5World();
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
  if (failed) throw failure;
  if (leaks.length > 0) throw new Error(`L5 world leaked:\n${leaks.join('\n')}`);
  return result as T;
}
