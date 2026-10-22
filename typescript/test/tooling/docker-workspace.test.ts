import {
  readFileSync,
  readdirSync,
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  rmSync,
  chmodSync,
  existsSync,
  statSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import {
  dependencyCacheKey,
  inspectDependencyCache,
  renderDependencyDockerfile,
} from '../../scripts/e2e-dependency-cache.mjs';
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

it('copies every external Cargo workspace member into the dependency cache planner', () => {
  const dockerfile = readFileSync(
    path.join(root, 'typescript/test/e2e/dependency-cache.Dockerfile'),
    'utf8'
  );
  const block = dockerfile
    .split(/^FROM /m)
    .find((part) => part.split('\n')[0].endsWith(' AS ci-planner'))!;
  const workdir = block.match(/^WORKDIR (\S+)$/m)![1];
  const external = crates.filter(({ manifest }) => !manifest.startsWith('rust/'));
  const missing = (text: string) =>
    external
      .filter(
        ({ manifest }) =>
          !copiedAt(text, manifest, workdir).includes(path.posix.join('/native', manifest))
      )
      .map(({ name }) => name);
  expect(external.length).toBeGreaterThan(0);
  expect(missing(block)).toEqual([]);
  for (const line of block
    .split('\n')
    .filter((line) => /^COPY (?:extensions|drivers)\//.test(line))) {
    const omitted = block.replace(`${line}\n`, '');
    const members = external
      .filter(({ manifest }) => copiedAt(line, manifest, workdir).length > 0)
      .map(({ name }) => name);
    expect(members.length).toBeGreaterThan(0);
    expect(missing(omitted)).toEqual(members);
  }
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
    `${file} [verification]: typescript/scripts/verify-native-installation.mjs -> typescript/scripts/migrated-state.mjs (expected /verification/typescript/scripts/migrated-state.mjs)`
  );
  expect(missing).toContain(
    `${file} [verification]: typescript/scripts/e2e-shards.mjs -> typescript/test/e2e/shard-weights.json (expected /verification/typescript/test/e2e/shard-weights.json)`
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
    `${file} [verification]: typescript/scripts/verify-native-artifact.mjs -> typescript/scripts/native-application-schema.mjs (expected /verification/typescript/scripts/native-application-schema.mjs)`
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
    if (/^RUN .*pnpm --filter tmt install|^RUN scripts\/build-native-artifact\.sh/.test(line)) {
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
  const files = dockerfiles.filter((file) => /pnpm --filter tmt install/.test(readSource(file)));
  expect(files.sort()).toEqual([
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
  const install = 'RUN cd typescript && pnpm --filter tmt install --frozen-lockfile\n';
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

it('keys E2E dependency archives by dependency bytes and auto-target paths, not ordinary source', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-dependency-key-'));
  const files = [
    'rust/Cargo.lock',
    'rust/rust-toolchain.toml',
    'rust/crates/sample/Cargo.toml',
    'rust/crates/sample/src/main.rs',
    'rust/crates/sample/src/module.rs',
    'typescript/scripts/e2e-dependency-cache.mjs',
    'typescript/test/e2e/dependency-cache.Dockerfile',
    'typescript/test/e2e/Dockerfile',
  ];
  try {
    for (const file of files) {
      mkdirSync(path.dirname(path.join(directory, file)), { recursive: true });
      writeFileSync(path.join(directory, file), file);
    }
    const key = dependencyCacheKey(directory, files);
    expect(key).toMatch(/^tmt-e2e-native-archive-v1-[a-f0-9]{64}$/);
    writeFileSync(path.join(directory, 'rust/crates/sample/src/main.rs'), 'new current source');
    expect(dependencyCacheKey(directory, files)).toBe(key);
    expect(dependencyCacheKey(directory, [...files, 'rust/crates/sample/src/ordinary.rs'])).toBe(
      key
    );
    expect(
      dependencyCacheKey(directory, [...files, 'rust/crates/sample/src/bin/tool/module.rs'])
    ).toBe(key);
    for (const target of [
      'build.rs',
      'src/bin/tool.rs',
      'src/bin/tool/main.rs',
      'examples/tool.rs',
      'tests/tool.rs',
      'benches/tool.rs',
    ]) {
      expect(dependencyCacheKey(directory, [...files, `rust/crates/sample/${target}`])).not.toBe(
        key
      );
    }
    writeFileSync(path.join(directory, 'rust/Cargo.lock'), 'new dependencies');
    expect(dependencyCacheKey(directory, files)).not.toBe(key);
    expect(() =>
      dependencyCacheKey(
        directory,
        files.filter((file) => !file.endsWith('Cargo.lock'))
      )
    ).toThrow('Missing E2E dependency input');
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

it('admits exact dependency payloads and refuses absent, mismatched, scratch and dummy payloads', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-dependency-payload-'));
  const key = 'tmt-e2e-native-archive-v1-' + 'a'.repeat(64);
  try {
    mkdirSync(path.join(directory, 'target/debug/deps'), { recursive: true });
    mkdirSync(path.join(directory, 'registry'), { recursive: true });
    writeFileSync(path.join(directory, 'cache-key'), key + '\n');
    writeFileSync(path.join(directory, 'target/debug/deps/libthirdparty.rlib'), 'retained library');
    writeFileSync(path.join(directory, 'registry/index'), 'registry bytes');
    expect(inspectDependencyCache(directory, key)).toEqual({
      usable: true,
      reason: 'exact dependency archive',
    });
    expect(inspectDependencyCache(directory, key.replace(/a$/, 'b'))).toEqual({
      usable: false,
      reason: 'dependency key differs',
    });
    mkdirSync(path.join(directory, 'target/tmp'));
    expect(inspectDependencyCache(directory, key)).toEqual({
      usable: false,
      reason: 'target contains non-dev scratch',
    });
    rmSync(path.join(directory, 'target/tmp'), { recursive: true });
    const dummy = path.join(directory, 'target/debug/deps/tmt_adapters-abcdef');
    writeFileSync(dummy, 'dummy');
    chmodSync(dummy, 0o755);
    expect(inspectDependencyCache(directory, key)).toEqual({
      usable: false,
      reason: 'dummy adapter executable retained',
    });
    rmSync(dummy);
    rmSync(path.join(directory, 'registry/index'));
    expect(inspectDependencyCache(directory, key)).toEqual({
      usable: false,
      reason: 'missing or empty registry',
    });
    expect(inspectDependencyCache(path.join(directory, 'absent'), key).usable).toBe(false);
    expect(inspectDependencyCache(directory, 'bad')).toEqual({
      usable: false,
      reason: 'invalid dependency key',
    });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

it('renders only two dependency COPYs while preserving the shipped native/runtime Dockerfile', () => {
  const text = readFileSync(path.join(root, 'typescript/test/e2e/Dockerfile'), 'utf8');
  const inserted =
    'COPY --from=tmtdeps /target/ /native/rust/target/\nCOPY --from=tmtdeps /registry/ /usr/local/cargo/registry/\n';
  const rendered = renderDependencyDockerfile(text);
  expect(rendered.replace(inserted, '')).toBe(text);
  expect(rendered.split(inserted)).toHaveLength(2);
  expect(rendered).toContain(inserted + 'RUN cargo build --locked --example tmux-probe');
  for (const invalid of [
    text.replace('AS native-tests', 'AS other'),
    text.replace('RUN cargo build --locked --example tmux-probe', 'RUN cargo changed'),
    text + '\nRUN cargo build --locked --example tmux-probe',
    text.replace('CARGO_INCREMENTAL=0', 'CARGO_INCREMENTAL=1'),
  ]) {
    expect(() => renderDependencyDockerfile(invalid)).toThrow(
      'E2E native Dockerfile anchor changed'
    );
  }
});

it('sanitizes only extensionless regular executable adapter dummies in the dependency seed', () => {
  const seed = readFileSync(
    path.join(root, 'typescript/test/e2e/dependency-cache.Dockerfile'),
    'utf8'
  );
  const command = seed.match(/&& (find target\/debug\/deps[^\n]+)$/m)![1];
  expect(seed).toContain('a3733ab416c3ffddd37914cd13919ca05fee1a1cf654f3016dcfe7f399d89cd1');
  expect(seed).not.toMatch(/cache-(?:from|to)|type=gha/);
  expect(seed).toContain('COPY --from=ci-cook /native/rust/target/debug/ /target/debug/');
  const directory = mkdtempSync(path.join(tmpdir(), 'tmt-dependency-sanitize-'));
  try {
    const deps = path.join(directory, 'target/debug/deps');
    mkdirSync(deps, { recursive: true });
    const retained = [
      'tmt_adapters-abcdef.rlib',
      'tmt_adapters-abcdef.rmeta',
      'tmt_adapters-abcdef.d',
      'thirdparty-abcdef',
      'tmt_adapters-nothex',
      'tmt_adapters-123456',
    ];
    for (const file of [...retained, 'tmt_adapters-abcdef']) {
      writeFileSync(path.join(deps, file), 'fixture');
      chmodSync(path.join(deps, file), file === 'tmt_adapters-123456' ? 0o644 : 0o755);
    }
    mkdirSync(path.join(deps, 'tmt_adapters-aaaaaa'));
    const result = spawnSync('sh', ['-c', command], {
      cwd: directory,
      encoding: 'utf8',
      timeout: 1000,
    });
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(0);
    expect(existsSync(path.join(deps, 'tmt_adapters-abcdef'))).toBe(false);
    expect(readdirSync(deps).sort()).toEqual([...retained, 'tmt_adapters-aaaaaa'].sort());
    for (const file of retained)
      expect(readFileSync(path.join(deps, file), 'utf8')).toBe('fixture');
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

it('keeps adapter selection after the shared native RUN and omits only the unused test compile', () => {
  const text = readFileSync(path.join(root, 'typescript/test/e2e/Dockerfile'), 'utf8');
  const lines = instructions(text);
  const native = lines.slice(
    0,
    lines.findIndex((line, index) => index > 0 && /^FROM\s/i.test(line))
  );
  const runs = native.filter((line) => /^RUN /i.test(line));
  expect(runs).toHaveLength(2);
  expect(runs[0]).not.toContain('TMT_E2E_ADAPTER_TESTS');
  expect(runs[0]).not.toContain('cargo test');
  expect(runs[0]).toContain('cp "target/$native_output/tmt-remote" /native-artifacts/tmt-remote');
  const arg = 'ARG TMT_E2E_ADAPTER_TESTS=1';
  expect(native.indexOf(arg)).toBeGreaterThan(native.indexOf(runs[0]));
  expect(native[native.indexOf(arg) + 1]).toBe(runs[1]);
  expect(text).toContain(
    'COPY --from=native-tests /native-artifacts/adapter-tests /opt/tmt-tests/adapter-tests'
  );
  const defaultFlag = /^ARG TMT_E2E_ADAPTER_TESTS=(\S+)$/m.exec(text)![1];
  for (const flag of [undefined, '1', '0', 'compile failure']) {
    const directory = mkdtempSync(path.join(tmpdir(), 'tmt-e2e-adapter-build-'));
    try {
      const bin = path.join(directory, 'bin');
      const artifacts = path.join(directory, 'artifacts');
      mkdirSync(bin);
      mkdirSync(artifacts);
      const cargo = path.join(bin, 'cargo');
      writeFileSync(
        cargo,
        `#!/usr/bin/env node
const fs = require('node:fs');
fs.appendFileSync('calls.jsonl', JSON.stringify(process.argv.slice(2)) + '\\n');
if (process.env.COMPILE_FAILURE === '1') process.exit(23);
fs.mkdirSync('target/debug/deps', { recursive: true });
fs.writeFileSync('target/debug/deps/tmt_adapters-aaaaaaaa', 'test-owned executable');
fs.chmodSync('target/debug/deps/tmt_adapters-aaaaaaaa', 0o755);
`
      );
      chmodSync(cargo, 0o755);
      // The fixture is mode 0755; map GNU find's executable predicate for BSD hosts.
      const find = path.join(bin, 'find');
      writeFileSync(
        find,
        `#!/usr/bin/env node
const args = process.argv.slice(2).flatMap(arg => arg === '-executable' ? ['-perm', '-111'] : [arg]);
const result = require('node:child_process').spawnSync('/usr/bin/find', args, { stdio: 'inherit' });
process.exit(result.status ?? 1);
`
      );
      chmodSync(find, 0o755);
      const result = spawnSync(
        'sh',
        ['-c', runs[1].slice(4).replaceAll('/native-artifacts', '"$ARTIFACTS"')],
        {
          cwd: directory,
          encoding: 'utf8',
          timeout: 10_000,
          env: {
            ...process.env,
            PATH: `${bin}${path.delimiter}${process.env.PATH ?? ''}`,
            ARTIFACTS: artifacts,
            TMT_E2E_ADAPTER_TESTS: flag === 'compile failure' ? '1' : (flag ?? defaultFlag),
            COMPILE_FAILURE: flag === 'compile failure' ? '1' : '0',
          },
        }
      );
      expect(result.error).toBeUndefined();
      expect(result.signal).toBeNull();
      expect(result.status, `${flag ?? 'default'}: ${result.stderr}`).toBe(
        flag === 'compile failure' ? 23 : 0
      );
      const calls = existsSync(path.join(directory, 'calls.jsonl'))
        ? readFileSync(path.join(directory, 'calls.jsonl'), 'utf8')
            .trim()
            .split('\n')
            .map((line) => JSON.parse(line))
        : [];
      expect(calls).toEqual(
        flag === '0' ? [] : [['test', '--locked', '--no-run', '-p', 'tmt-adapters']]
      );
      const artifact = path.join(artifacts, 'adapter-tests');
      if (flag === 'compile failure') expect(existsSync(artifact)).toBe(false);
      else if (flag === '0') {
        expect(result.stdout).toContain(
          'Skipping tmt-adapters test compile (TMT_E2E_ADAPTER_TESTS=0)'
        );
        expect(statSync(artifact).size).toBe(0);
        expect(statSync(artifact).mode & 0o111).toBe(0);
      } else {
        expect(readFileSync(artifact, 'utf8')).toBe('test-owned executable');
        expect(statSync(artifact).mode & 0o111).not.toBe(0);
      }
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }
});

it('keeps the host E2E bootstrap independent of installed packages', () => {
  const read = (file: string) => readFileSync(path.join(root, file), 'utf8');
  expect(
    imports('typescript/scripts/run-e2e.mjs', read('typescript/scripts/run-e2e.mjs')).filter(
      (module) => !module.startsWith('node:')
    )
  ).toEqual(['./e2e-dependency-cache.mjs']);
  expect(
    imports(
      'typescript/scripts/e2e-dependency-cache.mjs',
      read('typescript/scripts/e2e-dependency-cache.mjs')
    ).filter((module) => !module.startsWith('node:'))
  ).toEqual([]);
  expect(read('.dockerignore').split('\n')).toContain('**/node_modules');
  const image = read('typescript/test/e2e/Dockerfile');
  expect(image).toContain('RUN corepack prepare pnpm@10.33.0 --activate');
  expect(image).toContain('RUN cd typescript && pnpm --filter tmt install --frozen-lockfile');
});
