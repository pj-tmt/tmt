import { mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

/** Where design-review screenshots go: never a machine-specific path, so a spec passes on any host. */
export const captureDirectory = () =>
  process.env.COLAB_CAPTURE_DIR ?? path.join(tmpdir(), 'colab-app-captures');

/** A screenshot path in the capture directory, created on demand. */
export function capturePath(name: string) {
  const directory = captureDirectory();
  mkdirSync(directory, { recursive: true });
  return path.join(directory, name);
}
