import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vite-plus/test';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import { imports } from '../support/source-imports.js';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const { packages: crates } = readCargoWorkspace(root);
const nativeStages = [
  ['typescript/test/e2e/Dockerfile', 'native-tests', '/native'],
  ['typescript/test/native/artifact.Dockerfile', 'build', '/workspace'],
  ['extensions/tmt-office/typescript/services/office/Dockerfile', 'native-office', '/workspace'],
];

// Repository-context COPYs only: --from inputs come from another stage/image.
// Keep source-to-destination resolution shared with other embedded-input guards.
function copiedAt(text: string, input: string, workdir: string): string[] {
  return [...text.matchAll(/^COPY (.+)$/gm)].flatMap(([, line]) => {
    if (/--from(?:=|\s)/.test(line)) return [];
    const args = line.split(/\s+/).filter((arg) => !arg.startsWith('--'));
    const target = args.pop()!;
    return args.flatMap((arg) => {
      const source = arg.replace(/\/$/, '');
      if (input === source) {
        return [
          path.posix.resolve(
            workdir,
            target,
            target.endsWith('/') ? path.posix.basename(input) : ''
          ),
        ];
      }
      return input.startsWith(`${source}/`)
        ? [path.posix.resolve(workdir, target, input.slice(source.length + 1))]
        : [];
    });
  });
}

function rustFiles(directory: string): string[] {
  return readdirSync(path.join(root, directory), { withFileTypes: true }).flatMap((entry) => {
    const file = path.posix.join(directory, entry.name);
    if (entry.isDirectory()) return entry.name === 'target' ? [] : rustFiles(file);
    return entry.name.endsWith('.rs') ? [file] : [];
  });
}

function embeddedInputs(crate: (typeof crates)[number], withTests: boolean): string[] {
  return rustFiles(crate.dir).flatMap((source) => {
    if (!withTests && source.startsWith(`${crate.dir}/tests/`)) return [];
    const text = readFileSync(path.join(root, source), 'utf8');
    // Generated include_bytes! output in build scripts is not a source input.
    // Literal paths and manifest-relative concat! inputs are resolved as Rust does.
    const literal = [...text.matchAll(/include_(?:str|bytes)!\s*\(\s*"([^"\n]+)"/g)].map(
      ([, value]) => path.posix.normalize(path.posix.join(path.posix.dirname(source), value))
    );
    const manifest = [
      ...text.matchAll(
        /include_(?:str|bytes)!\s*\(\s*concat!\(\s*env!\("CARGO_MANIFEST_DIR"\),\s*"([^"\n]+)"/g
      ),
    ].map(([, value]) => path.posix.normalize(`${crate.dir}${value}`));
    return [...literal, ...manifest];
  });
}

it.each(nativeStages)('preserves every Cargo member in %s', (file, stage, destination) => {
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
          !copiedAt(text, manifest, workdir).includes(path.posix.join(destination, manifest))
      )
      .map(({ name }) => name);
  expect(missing(block!)).toEqual([]);
  const absent = block!.replace(/^COPY extensions\/tmt-remote\/rust.*\n/gm, '');
  expect(missing(absent)).toEqual(['tmt-remote']);
  const misplaced = block!.replace(/^(COPY extensions\/tmt-remote\/rust\/? )\S+$/m, '$1/wrong');
  expect(missing(misplaced)).toEqual(['tmt-remote']);
});

it.each(nativeStages)('preserves Rust embedded source inputs in %s', (file, stage, destination) => {
  const dockerfile = readFileSync(path.join(root, file), 'utf8');
  const block = dockerfile
    .split(/^FROM /m)
    .find((part) => part.split('\n')[0].endsWith(` AS ${stage}`))!;
  const workdir = block.match(/^WORKDIR (\S+)$/m)![1];
  // These stages preserve the full workspace. Check source/build inputs for every
  // member, plus integration-test inputs for packages whose test targets compile.
  const inputs = [
    ...new Set(
      crates.flatMap((crate) =>
        embeddedInputs(crate, new RegExp(`cargo test[^\\n]* -p ${crate.name}(?:\\s|$)`).test(block))
      )
    ),
  ];
  const missing = (text: string) =>
    inputs.filter(
      (input) => !copiedAt(text, input, workdir).includes(path.posix.join(destination, input))
    );
  expect(missing(block)).toEqual([]);
  const token = 'design/tokens/tokens.json';
  expect(inputs).toContain(token);
  const absent = block.replace(/^COPY design\/tokens\/tokens\.json.*\n/gm, '');
  expect(missing(absent)).toEqual([token]);
  const misplaced = block.replace(
    /^(COPY design\/tokens\/tokens\.json )\S+$/m,
    '$1/wrong/tokens.json'
  );
  expect(missing(misplaced)).toEqual([token]);
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

// test/support/cli-process launches every native scenario through the neutral caller fixture,
// so each image that runs native scenarios must place it where that module resolves it.
it('places the native caller fixture in the Office browser test image', () => {
  const dockerfile = readFileSync(
    path.join(root, 'extensions/tmt-office/typescript/services/office/Dockerfile'),
    'utf8'
  );
  const stage = (name: string) =>
    dockerfile.split(/^FROM /m).find((part) => part.split('\n')[0].endsWith(` AS ${name}`))!;
  expect(stage('native-office')).toMatch(
    /cargo \+\S+ build --locked --release -p tmt-adapters --example runtime-caller-fixture/
  );
  expect(stage('browser-tests')).toContain(
    'COPY --from=native-office --chown=node:node /workspace/rust/target/release/examples/runtime-caller-fixture /workspace/rust/target/debug/examples/runtime-caller-fixture'
  );
  expect(readFileSync(path.join(root, 'typescript/test/support/cli-process.ts'), 'utf8')).toContain(
    "'../../../rust/target/debug/examples/runtime-caller-fixture'"
  );
});

it('places the native caller fixture in the Docker E2E image', () => {
  const dockerfile = readFileSync(path.join(root, 'typescript/test/e2e/Dockerfile'), 'utf8');
  // native-tests copies the runtime-caller-fixture build to /native-artifacts/codex.
  expect(dockerfile).toContain(
    'cp target/debug/examples/runtime-caller-fixture /native-artifacts/codex'
  );
  expect(dockerfile).toContain(
    'COPY --from=native-tests /native-artifacts/codex /workspace/rust/target/debug/examples/runtime-caller-fixture'
  );
});
