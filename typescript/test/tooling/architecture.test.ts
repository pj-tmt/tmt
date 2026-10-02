import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';
import { imports } from '../support/source-imports.js';

const workspaceRoot = fileURLToPath(new URL('../../', import.meta.url));
const repositoryRoot = fileURLToPath(new URL('../../../', import.meta.url));
const sourceRoot = path.join(workspaceRoot, 'src');
const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);
const compilerOptions = ts.parseJsonConfigFileContent(
  ts.readConfigFile(path.join(workspaceRoot, 'tsconfig.json'), ts.sys.readFile).config,
  ts.sys,
  workspaceRoot
).options;
const resolutionCache = ts.createModuleResolutionCache(
  repositoryRoot,
  (file) => file,
  compilerOptions
);

function repositoryPath(file: string): string {
  return path.relative(repositoryRoot, file).split(path.sep).join('/');
}

function suiteImportViolations(file: string, text: string): string[] {
  const from = repositoryPath(file);
  const fromSuite = /^typescript\/test\/(support|native|e2e|tooling|stress)\//.exec(from)?.[1];
  const harness = 'typescript/test/e2e/harness/';
  const facade = 'typescript/test/e2e/harness.ts';
  const fixture = `${harness}fixture.ts`;
  const harnessHelper = from.startsWith(harness) && from !== fixture;
  const scenario =
    (fromSuite === 'e2e' && !from.startsWith(harness) && from !== facade) ||
    (from.startsWith('extensions/') && from.includes('/e2e/'));
  return imports(text).flatMap((specifier) => {
    const resolved = ts.resolveModuleName(
      specifier,
      file,
      compilerOptions,
      ts.sys,
      resolutionCache
    ).resolvedModule;
    if (!resolved) return [];
    const to = repositoryPath(resolved.resolvedFileName);
    const toSuite = /^typescript\/test\/(support|native|e2e|tooling|stress)\//.exec(to)?.[1];
    let action: string | undefined;
    if (fromSuite === 'support' && toSuite && toSuite !== 'support') {
      action = 'move cross-suite behavior to test/support; shared support cannot import a suite';
    } else if (
      (fromSuite === 'native' || fromSuite === 'e2e') &&
      (toSuite === 'native' || toSuite === 'e2e' || toSuite === 'tooling') &&
      fromSuite !== toSuite
    ) {
      action = 'keep suite-only behavior with its suite or move shared behavior to test/support';
    } else if (harnessHelper && to === fixture) {
      action = 'pass fixture-owned state into the helper; harness helpers cannot import fixture.ts';
    } else if (scenario && to.startsWith(harness)) {
      action = 'import typescript/test/e2e/harness.ts instead of a private harness module';
    }
    return action ? [`${from} -> ${to}: ${action}`] : [];
  });
}

function retainedTestFiles(directory: string): string[] {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) return retainedTestFiles(file);
    return /\.(?:ts|mjs)$/.test(entry.name) ? [file] : [];
  });
}

function legacyTestImports(file: string, text: string): string[] {
  return imports(text).filter((specifier) => {
    if (!specifier.startsWith('.') && !path.isAbsolute(specifier)) return false;
    const target = path.resolve(path.dirname(file), specifier);
    const relative = path.relative(sourceRoot, target);
    return relative === '' || (!relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative));
  });
}

describe('retained developer tooling boundaries', () => {
  it('has one native product runtime and a private developer-only Node package', () => {
    const packageMetadata = JSON.parse(
      fs.readFileSync(path.join(workspaceRoot, 'package.json'), 'utf8')
    );
    expect(packageMetadata.private).toBe(true);
    expect(packageMetadata.bin).toBeUndefined();
    expect(packageMetadata.files).toBeUndefined();
    expect(packageMetadata.dependencies).toBeUndefined();
    expect(fs.existsSync(sourceRoot)).toBe(false);
    expect(fs.existsSync(path.join(repositoryRoot, 'src'))).toBe(false);
    expect(fs.existsSync(path.join(workspaceRoot, 'bin', 'tmux-team'))).toBe(false);
    expect(fs.existsSync(path.join(repositoryRoot, 'bin', 'tmux-team'))).toBe(false);
    expect(
      fs.existsSync(path.join(repositoryRoot, 'rust', 'crates', 'tmt-cli', 'src', 'main.rs'))
    ).toBe(true);
  });

  it('rejects legacy runtime imports from retained native test infrastructure', () => {
    const file = path.join(sourceRoot, '../test/native/example.ts');
    for (const form of [
      "import { open } from '../../src/storage/sqlite-adapter.js';",
      "export * from '../../src/config.js';",
      "const legacy = import('../../src/context.js');",
      "const legacy = require('../../src/context.js');",
      "type Legacy = import('../../src/context.js').Context;",
    ])
      expect(legacyTestImports(file, form)).toHaveLength(1);
    expect(legacyTestImports(file, "import { runCli } from '../support/cli-process.js';")).toEqual(
      []
    );
    expect(legacyTestImports(file, "import Database from 'better-sqlite3';")).toEqual([]);
    expect(legacyTestImports(file, "// import legacy from '../../src/context.js';")).toEqual([]);
  });

  it('rejects suite crossings in every supported literal import form', () => {
    const file = path.join(repositoryRoot, 'typescript/test/native/__fixture__.ts');
    for (const form of [
      "import { expectJsonResult as check } from '../e2e/cli-assertions.js';",
      "export * from '../e2e/cli-assertions.js';",
      "const helper = import('../e2e/cli-assertions.js');",
      "const helper = require('../e2e/cli-assertions.js');",
      "type Result = import('../e2e/harness.js').CliResult;",
      "import helper = require('../e2e/cli-assertions.js');",
      'const helper = import(`../e2e/cli-assertions.js`);',
      'const helper = require(`../e2e/cli-assertions.js`);',
      "const helper = import('../e2e/harness.js', { with: { type: 'json' } });",
    ]) {
      expect(suiteImportViolations(file, form), form).toHaveLength(1);
    }
    for (const form of [
      "// import '../e2e/cli-assertions.js';",
      `const text = 'require("../e2e/cli-assertions.js")';`,
      'void import(`../e2e/${name}.js`);',
      'require(specifier);',
    ]) {
      expect(suiteImportViolations(file, form), form).toEqual([]);
    }
  });

  it('rejects shared-support and suite direction violations with an actionable message', () => {
    for (const [from, specifier] of [
      ['support', '../native/cli.test.js'],
      ['support', '../e2e/harness.js'],
      ['support', '../tooling/cli-process.test.js'],
      ['support', '../stress/native-installation-capacity.test.js'],
      ['native', '../tooling/cli-process.test.js'],
      ['e2e', '../native/cli.test.js'],
      ['e2e', '../tooling/cli-process.test.js'],
    ]) {
      const file = path.join(repositoryRoot, `typescript/test/${from}/__fixture__.ts`);
      expect(
        suiteImportViolations(file, `import '${specifier}';`),
        `${from} -> ${specifier}`
      ).toHaveLength(1);
    }
    expect(
      suiteImportViolations(
        path.join(repositoryRoot, 'typescript/test/support/__fixture__.ts'),
        "import '../native/cli.test.js';"
      )
    ).toEqual([
      'typescript/test/support/__fixture__.ts -> typescript/test/native/cli.test.ts: move cross-suite behavior to test/support; shared support cannot import a suite',
    ]);
  });

  it('keeps the fixture below its facade and out of harness helpers', () => {
    const helper = path.join(repositoryRoot, 'typescript/test/e2e/harness/__fixture__.ts');
    expect(
      suiteImportViolations(helper, "export { E2EFixture as Hidden } from './fixture.js';")
    ).toEqual([
      'typescript/test/e2e/harness/__fixture__.ts -> typescript/test/e2e/harness/fixture.ts: pass fixture-owned state into the helper; harness helpers cannot import fixture.ts',
    ]);
    for (const [file, specifier] of [
      ['typescript/test/e2e/__fixture__.ts', './harness/fixture.js'],
      ['typescript/test/e2e/__fixture__.ts', './harness/types.js'],
      [
        'extensions/tmt-office/typescript/apps/office/e2e/__fixture__.ts',
        '../../../../../../typescript/test/e2e/harness/fixture.js',
      ],
    ]) {
      expect(
        suiteImportViolations(path.join(repositoryRoot, file), `import '${specifier}';`)
      ).toHaveLength(1);
    }
  });

  it('allows focused tooling tests and the documented harness and support directions', () => {
    for (const [file, specifier] of [
      ['typescript/test/tooling/cli-assertions.test.ts', '../e2e/cli-assertions.js'],
      ['typescript/test/tooling/wait-for-file.test.ts', '../e2e/wait-for-file.js'],
      ['typescript/test/e2e/harness.ts', './harness/fixture.js'],
      ['typescript/test/e2e/harness/fixture.ts', './readiness.js'],
      ['typescript/test/e2e/harness/cleanup.ts', './types.js'],
      ['typescript/test/e2e/__fixture__.ts', './harness.js'],
      ['typescript/test/native/__fixture__.ts', '../support/cli-process.js'],
      [
        'extensions/tmt-office/typescript/apps/office/e2e/__fixture__.ts',
        '../../../../../../typescript/test/e2e/harness.js',
      ],
      [
        'extensions/tmt-office/typescript/apps/office/e2e/__fixture__.ts',
        '../../../../../../typescript/test/support/cli-process.js',
      ],
      [
        'extensions/tmt-office/typescript/apps/office/e2e/__fixture__.ts',
        '../../../services/office/functions/test/emulator-fixture.js',
      ],
      [
        'extensions/tmt-office/typescript/apps/office/e2e/__fixture__.ts',
        '../../../services/office/functions/src',
      ],
    ]) {
      // Resolve positive controls too: missing modules must not make them pass.
      const source = path.join(repositoryRoot, file);
      expect(
        ts.resolveModuleName(specifier, source, compilerOptions, ts.sys).resolvedModule,
        `${file} -> ${specifier}`
      ).toBeDefined();
      expect(suiteImportViolations(source, `export * from '${specifier}';`)).toEqual([]);
    }
  });

  it('keeps current test imports within their suite and harness owners', () => {
    const files: string[] = runPackedCommand(
      'git',
      ['ls-files', '-z', '--', 'typescript/test', 'extensions'],
      { cwd: repositoryRoot, env: process.env }
    )
      .split('\0')
      .filter((file: string) => /\.(?:[cm]?ts|tsx|[cm]?js)$/.test(file));
    expect(files).toContain('typescript/test/support/cli-process.ts');
    expect(files).toContain('typescript/test/e2e/harness/fixture.ts');
    expect(files.some((file) => file.startsWith('extensions/') && file.includes('/e2e/'))).toBe(
      true
    );
    expect(
      files.flatMap((file) => {
        const absolute = path.join(repositoryRoot, file);
        return suiteImportViolations(absolute, fs.readFileSync(absolute, 'utf8'));
      })
    ).toEqual([]);
  });

  it('keeps all retained test modules independent of the TypeScript product', () => {
    const directories = ['native', 'e2e', 'support', 'tooling'].map((name) =>
      path.join(sourceRoot, '../test', name)
    );
    directories.push(path.join(sourceRoot, '../scripts'));
    const files = directories.flatMap(retainedTestFiles);
    expect(files.some((file) => file.endsWith('/storage-fixture.ts'))).toBe(true);
    expect(files.some((file) => file.endsWith('/cli-process.ts'))).toBe(true);
    expect(
      files.flatMap((file) =>
        legacyTestImports(file, fs.readFileSync(file, 'utf8')).map(
          (specifier) => `${file} -> ${specifier}`
        )
      )
    ).toEqual([]);
  });
});
