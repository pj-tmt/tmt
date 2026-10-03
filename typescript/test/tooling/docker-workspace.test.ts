import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vite-plus/test';
import { readWorkspace } from '../../scripts/release-please-config.mjs';
import { imports } from '../support/source-imports.js';

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

it.each([
  ['extensions/tmt-office/typescript/apps/office/Dockerfile', 'apps/office/vite.config.ts'],
  [
    'extensions/tmt-office/typescript/services/office/Dockerfile',
    'services/office/functions/vitest.config.ts',
  ],
])('preserves shared lint inputs in %s', (file, config) => {
  const configPath = `extensions/tmt-office/typescript/${config}`;
  const module = imports(readFileSync(path.join(root, configPath), 'utf8')).find((name) =>
    name.endsWith('/lint-config.mjs')
  );
  expect(module).toBeDefined();
  const source = path.posix.normalize(path.posix.join(path.posix.dirname(configPath), module!));
  const inputs = [source, source.replace(/\.mjs$/, '.d.mts')];
  const dockerfile = readFileSync(path.join(root, file), 'utf8');
  const missing = (text: string) => {
    const copied = new Map<string, string>();
    const beforeChecks = text.split(/^RUN .*\bcheck\b/m)[0];
    for (const line of beforeChecks.split('\n').filter((value) => value.startsWith('COPY '))) {
      const args = line
        .split(/\s+/)
        .slice(1)
        .filter((value) => !value.startsWith('--'));
      const destination = args.pop()!;
      for (const input of args) {
        copied.set(input, path.posix.join(destination, path.posix.basename(input)));
      }
    }
    return inputs.filter((input) => copied.get(input) !== `/workspace/${input}`);
  };
  expect(missing(dockerfile)).toEqual([]);
  const absent = dockerfile.replace(/^COPY .*lint-config\.mjs.*\n/gm, '');
  expect(missing(absent)).toEqual(inputs);
  const misplaced = dockerfile.replace(/\/workspace\/typescript\/scripts\//g, '/wrong/');
  expect(missing(misplaced)).toEqual(inputs);
});
