import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import { readWorkspace } from '../../scripts/release-please-config.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const { crates } = readWorkspace(root);
it.each([
  ['typescript/test/e2e/Dockerfile', 'native-tests', '/native'],
  ['typescript/test/native/artifact.Dockerfile', 'build', '/workspace'],
  ['extensions/tmt-office/typescript/services/office/Dockerfile', 'native-office', '/workspace'],
])('preserves every Cargo member in %s', (file, stage, destination) => {
  const dockerfile = readFileSync(path.join(root, file), 'utf8');
  const block = dockerfile
    .split(/^FROM /m)
    .find((part) => part.split('\n')[0].endsWith(` AS ${stage}`));
  expect(block, stage).toBeDefined();
  const workdir = block!.match(/^WORKDIR (\S+)$/m)![1];
  const missing = (text: string) =>
    crates
      .filter(
        ({ manifest }) =>
          ![...text.matchAll(/^COPY ([^ -]\S*) (\S+)$/gm)].some(([, source, target]) => {
            source = source.replace(/\/$/, '');
            return (
              manifest.startsWith(`${source}/`) &&
              path.posix.resolve(workdir, target, manifest.slice(source.length + 1)) ===
                path.posix.join(destination, manifest)
            );
          })
      )
      .map(({ name }) => name);
  expect(missing(block!)).toEqual([]);
  const absent = block!.replace(/^COPY extensions\/tmt-remote\/rust.*\n/gm, '');
  expect(missing(absent)).toEqual(['tmt-remote']);
  const misplaced = block!.replace(/^(COPY extensions\/tmt-remote\/rust\/? )\S+$/m, '$1/wrong');
  expect(missing(misplaced)).toEqual(['tmt-remote']);
});
