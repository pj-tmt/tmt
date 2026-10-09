import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vite-plus/test';
import ts from 'typescript';
import { readCargoWorkspace } from '../../scripts/cargo-workspace.mjs';
import { imports } from '../support/source-imports.js';

const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);

const root = fileURLToPath(new URL('../../../', import.meta.url));
// COPY guards inspect workspace sources, not external dependency packages.
// --no-deps also keeps Code quality independent of a warmed native Cargo cache.
const { packages: crates } = readCargoWorkspace(root, {
  runner: (
    command: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; timeoutMs: number }
  ) => runPackedCommand(command, [...args, '--no-deps'], options),
});
const nativeStages = [
  ['typescript/test/e2e/Dockerfile', 'native-tests', '/native'],
  ['typescript/test/native/artifact.Dockerfile', 'build', '/workspace'],
  ['extensions/tmt-office/typescript/services/office/Dockerfile', 'native-office', '/workspace'],
];

type Copy = { sources: string[]; target: string; workdir: string };

// Share repository-context COPY parsing and destination resolution across guards.
// --from inputs belong to another stage/image, not the repository build context.
function copyInstruction(line: string, workdir: string): Copy | undefined {
  if (!/^COPY\s/i.test(line) || /--from(?:=|\s)/.test(line)) return;
  const argumentsText = line.replace(/^COPY\s+(?:--\S+\s+)*/i, '');
  const args: string[] = argumentsText.startsWith('[')
    ? JSON.parse(argumentsText)
    : argumentsText.split(/\s+/);
  const target = args.pop()!;
  return {
    sources: args.map((source) => path.posix.normalize(source).replace(/\/$/, '')),
    target,
    workdir,
  };
}

function instructions(text: string): string[] {
  return text
    .replace(/\\\r?\n\s*/g, ' ')
    .split(/\r?\n/)
    .map((line) => line.trim());
}

function copyLocations(copies: Copy[], input: string): string[] {
  return copies.flatMap(({ sources, target, workdir }) =>
    sources.flatMap((source) => {
      if (input === source) {
        return [
          path.posix.resolve(
            workdir,
            target,
            target.endsWith('/') || sources.length > 1 ? path.posix.basename(input) : ''
          ),
        ];
      }
      const prefix = source === '.' ? '' : `${source}/`;
      return input.startsWith(prefix)
        ? [path.posix.resolve(workdir, target, input.slice(prefix.length))]
        : [];
    })
  );
}

function copiedAt(text: string, input: string, workdir: string): string[] {
  return copyLocations(
    instructions(text).flatMap((line) => {
      const copy = copyInstruction(line, workdir);
      return copy ? [copy] : [];
    }),
    input
  );
}

function dockerStages(text: string): { name: string; copies: Copy[] }[] {
  const stages: { name: string; copies: Copy[]; workdir: string }[] = [];
  for (const line of instructions(text)) {
    const from = line.match(/^FROM\s+(?:--\S+\s+)*(\S+)(?:\s+AS\s+(\S+))?$/i);
    if (from) {
      const parent = stages.find(({ name }) => name === from[1]);
      stages.push({
        name: from[2] ?? String(stages.length),
        copies: [...(parent?.copies ?? [])],
        workdir: parent?.workdir ?? '/',
      });
      continue;
    }
    const stage = stages.at(-1);
    if (!stage) continue;
    const workdir = line.match(/^WORKDIR\s+(\S+)$/i);
    if (workdir) stage.workdir = path.posix.resolve(stage.workdir, workdir[1]);
    const copy = copyInstruction(line, stage.workdir);
    if (copy) stage.copies.push(copy);
  }
  return stages;
}

// Static imports/re-exports and literal repository URL inputs only; dynamic imports are out of scope.
function scriptInputs(text: string): { value: string; module: boolean }[] {
  const inputs: { value: string; module: boolean }[] = [];
  const source = ts.createSourceFile('module.mjs', text, ts.ScriptTarget.Latest, true);
  const collect = (node: ts.Node): void => {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteralLike(node.moduleSpecifier) &&
      /^\.\.?\//.test(node.moduleSpecifier.text)
    ) {
      inputs.push({ value: node.moduleSpecifier.text, module: true });
    }
    if (
      ts.isNewExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === 'URL' &&
      node.arguments?.length === 2 &&
      ts.isStringLiteralLike(node.arguments[0]) &&
      ts.isPropertyAccessExpression(node.arguments[1]) &&
      node.arguments[1].name.text === 'url' &&
      ts.isMetaProperty(node.arguments[1].expression) &&
      node.arguments[1].expression.keywordToken === ts.SyntaxKind.ImportKeyword &&
      node.arguments[1].expression.name.text === 'meta' &&
      !/^(?:[a-z][a-z0-9+.-]*:|\/)/i.test(node.arguments[0].text)
    ) {
      inputs.push({ value: node.arguments[0].text, module: false });
    }
    ts.forEachChild(node, collect);
  };
  collect(source);
  return inputs;
}

function missingScriptInputs(
  dockerfile: string,
  text: string,
  files: string[],
  read: (file: string) => string
): string[] {
  const repositoryFiles = new Set(files);
  const missing = new Set<string>();
  for (const stage of dockerStages(text)) {
    const pending = files
      .filter((file) => file.endsWith('.mjs'))
      .flatMap((file) => copyLocations(stage.copies, file).map((location) => ({ file, location })));
    const seen = new Set<string>();
    while (pending.length) {
      const { file, location } = pending.pop()!;
      const key = `${file}:${location}`;
      if (seen.has(key)) continue;
      seen.add(key);
      for (const { value: input, module } of scriptInputs(read(file))) {
        const dependency = path.posix.normalize(path.posix.join(path.posix.dirname(file), input));
        // URL references to directories or generated binaries are not repository file inputs.
        if (!module && !repositoryFiles.has(dependency)) continue;
        const expected = path.posix.resolve(path.posix.dirname(location), input);
        if (
          !repositoryFiles.has(dependency) ||
          !copyLocations(stage.copies, dependency).includes(expected)
        ) {
          missing.add(
            `${dockerfile} [${stage.name}]: ${file} -> ${dependency} (expected ${expected})`
          );
        }
        if (module && repositoryFiles.has(dependency) && /\.[cm]?js$/.test(dependency))
          pending.push({ file: dependency, location: expected });
      }
    }
  }
  return [...missing].sort();
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

it('sets disposable native fixture profiles before Cargo runs in the Docker E2E image', () => {
  const dockerfile = readFileSync(path.join(root, 'typescript/test/e2e/Dockerfile'), 'utf8');
  const setting = 'ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0';
  const configured = (text: string) => {
    const lines = instructions(text);
    const nextStage = lines.findIndex((line, index) => index > 0 && /^FROM\s/i.test(line));
    const native = lines.slice(0, nextStage);
    const cargo = native.findIndex((line) => /^RUN\s.*\bcargo\b/.test(line));
    const settings = native.filter((line) =>
      /^(?:ENV|ARG)\s.*CARGO_(?:PROFILE_DEV_DEBUG|INCREMENTAL)/.test(line)
    );
    return (
      / AS native-tests$/i.test(native[0]) &&
      settings.length === 1 &&
      settings[0] === setting &&
      native.indexOf(setting) < cargo &&
      lines
        .slice(nextStage)
        .every((line) => !/^(?:ENV|ARG)\s.*CARGO_(?:PROFILE_DEV_DEBUG|INCREMENTAL)/.test(line))
    );
  };
  expect(configured(dockerfile)).toBe(true);
  expect(configured(dockerfile.replace(`${setting}\n`, ''))).toBe(false);
  expect(
    configured(
      dockerfile.replace(`${setting}\n`, '').replace(/^FROM node:/m, `${setting}\nFROM node:`)
    )
  ).toBe(false);
  expect(
    configured(
      dockerfile.replace(`${setting}\n`, '').replace(/^(FROM node:.*)$/m, `$1\n${setting}`)
    )
  ).toBe(false);
  expect(
    configured(dockerfile.replace('CARGO_PROFILE_DEV_DEBUG=0', 'CARGO_PROFILE_DEV_DEBUG=1'))
  ).toBe(false);
  expect(configured(dockerfile.replace('CARGO_INCREMENTAL=0', 'CARGO_INCREMENTAL=1'))).toBe(false);
});

const trackedFiles: string[] = runPackedCommand('git', ['ls-files', '-z'], {
  cwd: root,
  env: process.env,
})
  .split('\0')
  .filter(Boolean);
const dockerfiles = trackedFiles.filter((file) => /(?:^|\/|\.)Dockerfile$/.test(file));
const readSource = (file: string) => readFileSync(path.join(root, file), 'utf8');
it.each(dockerfiles)('preserves copied script import and file-read closure in %s', (file) => {
  expect(missingScriptInputs(file, readSource(file), trackedFiles, readSource)).toEqual([]);
});

it('rejects the Office script-only COPY from before #1680', () => {
  const file = 'extensions/tmt-office/typescript/services/office/Dockerfile';
  const current = readSource(file);
  expect(missingScriptInputs(file, current, trackedFiles, readSource)).toEqual([]);
  const broken = current.replace(
    /^COPY --chown=node:node typescript\/scripts\/native-artifact-policy\.mjs .*$/m,
    'COPY --chown=node:node typescript/scripts/native-artifact-policy.mjs /workspace/typescript/scripts/'
  );
  expect(broken).not.toBe(current);
  expect(missingScriptInputs(file, broken, trackedFiles, readSource)).toContain(
    `${file} [browser-tests-base]: typescript/scripts/native-artifact-policy.mjs -> typescript/scripts/component-skills.mjs (expected /workspace/typescript/scripts/component-skills.mjs)`
  );
});

it('rejects missing artifact verifier imports and transitive URL file inputs', () => {
  const file = 'typescript/test/native/artifact.Dockerfile';
  const current = readSource(file);
  const broken = current.replace(
    /^COPY typescript\/(?:scripts\/migrated-state\.mjs|test\/e2e\/shard-weights\.json).*\n/gm,
    ''
  );
  expect(broken).not.toBe(current);
  const missing = missingScriptInputs(file, broken, trackedFiles, readSource);
  expect(missing).toContain(
    `${file} [1]: typescript/scripts/verify-native-installation.mjs -> typescript/scripts/migrated-state.mjs (expected /verification/typescript/scripts/migrated-state.mjs)`
  );
  expect(missing).toContain(
    `${file} [1]: typescript/scripts/e2e-shards.mjs -> typescript/test/e2e/shard-weights.json (expected /verification/typescript/test/e2e/shard-weights.json)`
  );
});

it('rejects removal of the compiled schema verifier import from the artifact image', () => {
  const file = 'typescript/test/native/artifact.Dockerfile';
  const current = readSource(file);
  const broken = current.replace(
    /^COPY typescript\/scripts\/native-application-schema\.mjs.*\n/m,
    ''
  );
  expect(broken).not.toBe(current);
  expect(missingScriptInputs(file, broken, trackedFiles, readSource)).toContain(
    `${file} [1]: typescript/scripts/verify-native-artifact.mjs -> typescript/scripts/native-application-schema.mjs (expected /verification/typescript/scripts/native-application-schema.mjs)`
  );
});

it('checks transitive imports, cycles, URL reads and image destinations within each stage', () => {
  const sources: Record<string, string> = {
    'scripts/main.mjs': "import './nested/helper.mjs'; import 'node:fs'; // import './ignored.mjs'",
    'scripts/nested/helper.mjs': `export { value } from '../value.js';
      new URL('../../data/policy.json', import.meta.url);
      new URL('policy.json', import.meta.url);
      new URL('../../../rust/target/debug/tmt', import.meta.url);
      import('./optional.mjs');
      new URL('../../data/', import.meta.url);
      new URL('../../unrelated.json', other.url);`,
    'scripts/nested/policy.json': '{}',
    'scripts/value.js': "import './main.mjs'; export const value = 1;",
    'data/policy.json': '{}',
  };
  const check = (text: string) =>
    missingScriptInputs('fixture.Dockerfile', text, Object.keys(sources), (file) => sources[file]);
  const complete = String.raw`FROM node AS base
WORKDIR /workspace
COPY --chown=node:node \
["scripts/", "scripts/"]
COPY data/ data/
FROM base AS child
WORKDIR scripts
`;
  expect(check(complete)).toEqual([]);
  expect(check('FROM node\nWORKDIR /workspace\nCOPY . .')).toEqual([]);
  expect(check(complete.replace('COPY data/ data/', 'COPY data/ elsewhere/'))).toEqual([
    'fixture.Dockerfile [base]: scripts/nested/helper.mjs -> data/policy.json (expected /workspace/data/policy.json)',
    'fixture.Dockerfile [child]: scripts/nested/helper.mjs -> data/policy.json (expected /workspace/data/policy.json)',
  ]);
  expect(
    check(`FROM node AS first
WORKDIR /workspace
COPY scripts/ scripts/
FROM node AS second
WORKDIR /workspace
COPY data/ data/`)
  ).toEqual([
    'fixture.Dockerfile [first]: scripts/nested/helper.mjs -> data/policy.json (expected /workspace/data/policy.json)',
  ]);
  expect(
    check(`FROM node
WORKDIR /workspace
COPY scripts/main.mjs scripts/nested/helper.mjs scripts/
COPY --from=other /data/policy.json data/policy.json`)
  ).toContain(
    'fixture.Dockerfile [0]: scripts/nested/helper.mjs -> scripts/value.js (expected /workspace/scripts/value.js)'
  );
});

// E2E/artifact prepare the admitted static input. Office has no reader yet and
// remains covered by the actual embedded-input closure above, without speculative COPY.
it('preserves prepared browser CSS in E2E and artifact native stages', () => {
  const input = 'design/browser-ui/generated/static.css';
  for (const [file, name, destination] of nativeStages.slice(0, 2)) {
    const text = readSource(file);
    const locations = (source: string) => {
      const stage = dockerStages(source).find((stage) => stage.name === name);
      return stage ? copyLocations(stage.copies, input) : [];
    };
    const expected = `${destination}/${input}`;
    expect(locations(text)).toContain(expected);
    const stripped = text
      .split('\n')
      .filter((line) => !line.startsWith(`COPY ${input} `))
      .join('\n');
    expect(locations(stripped)).not.toContain(expected);
    const misplaced = text.replace(
      new RegExp(`COPY ${input} [^\n]+`),
      `COPY ${input} /wrong/static.css`
    );
    expect(locations(misplaced)).not.toContain(expected);
  }
});

// A filtered root install still reads the registered leaf's manifest. Inspect
// only inputs available before that RUN, not later COPY or another stage.
function missingBrowserManifest(text: string): string[] {
  const failures: string[] = [];
  const input = 'design/browser-ui/package.json';
  let prefix = '';
  for (const line of instructions(text)) {
    if (
      /^RUN .*pnpm --filter tmux-team install|^RUN scripts\/build-native-artifact\.sh/.test(line)
    ) {
      const stage = dockerStages(prefix).at(-1)!;
      const workspace = copyLocations(stage.copies, 'typescript/pnpm-workspace.yaml');
      expect(workspace.length, 'workspace input for filtered root install').toBeGreaterThan(0);
      for (const location of workspace) {
        const expected = path.posix.resolve(path.posix.dirname(location), '../', input);
        if (!copyLocations(stage.copies, input).includes(expected))
          failures.push(`${stage.name}: ${expected}`);
      }
    }
    prefix += `${line}\n`;
  }
  return failures;
}

it('copies the new workspace manifest before filtered root installs and artifact preparation', () => {
  const files = dockerfiles.filter((file) =>
    /pnpm --filter tmux-team install/.test(readSource(file))
  );
  expect(files.sort()).toEqual([
    'extensions/tmt-office/typescript/services/office/Dockerfile',
    'typescript/test/e2e/Dockerfile',
    'typescript/test/native/artifact.Dockerfile',
  ]);
  for (const file of files) {
    const text = readSource(file);
    expect(missingBrowserManifest(text), file).toEqual([]);
    const absent = text.replace(/^COPY .*design\/browser-ui\/package\.json.*\n/gm, '');
    expect(missingBrowserManifest(absent).length, file).toBeGreaterThan(0);
    const misplaced = text.replace(
      /^(COPY .*design\/browser-ui\/package\.json) \S+$/gm,
      '$1 /wrong/package.json'
    );
    expect(missingBrowserManifest(misplaced).length, file).toBeGreaterThan(0);
  }
});

it('refuses late and cross-stage workspace manifest copies', () => {
  const prefix = `FROM node AS base
WORKDIR /workspace
COPY typescript/pnpm-workspace.yaml typescript/
`;
  const copy = 'COPY design/browser-ui/package.json design/browser-ui/package.json\n';
  const install = 'RUN cd typescript && pnpm --filter tmux-team install --frozen-lockfile\n';
  expect(missingBrowserManifest(prefix + copy + install)).toEqual([]);
  expect(missingBrowserManifest(prefix + install + copy)).toEqual([
    'base: /workspace/design/browser-ui/package.json',
  ]);
  expect(
    missingBrowserManifest(
      prefix +
        copy +
        'FROM node AS other\nWORKDIR /workspace\nCOPY typescript/pnpm-workspace.yaml typescript/\n' +
        install
    )
  ).toEqual(['other: /workspace/design/browser-ui/package.json']);
});
