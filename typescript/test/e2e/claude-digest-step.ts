import { randomUUID } from 'node:crypto';
import fs from 'node:fs';

/** Publish one complete command before the mock's existence-based poll can see it. */
export async function publishDigestStep(
  log: string,
  step: { event: string; [key: string]: unknown }
): Promise<void> {
  const command = `${log}.digest-step`;
  const staged = `${command}.${randomUUID()}.tmp`;
  try {
    await fs.promises.writeFile(staged, JSON.stringify(step), { flag: 'wx', mode: 0o600 });
    await fs.promises.rename(staged, command);
  } finally {
    await fs.promises.rm(staged, { force: true });
  }
}
