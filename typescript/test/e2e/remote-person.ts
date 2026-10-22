import { expect } from 'vite-plus/test';
import type { E2EFixture } from './harness.js';

// The person Core attributes an accepted Remote send to (#4255). Setting it per fixture keeps the
// wake line independent of the login name of whoever runs the suite.
export const PERSON = 'e2e-person';

export async function namePerson(fixture: E2EFixture): Promise<void> {
  expect((await fixture.runJsonCli(['config', 'set', 'user.name', PERSON, '--global'])).code).toBe(
    0
  );
}
