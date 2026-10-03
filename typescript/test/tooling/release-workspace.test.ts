import { describe, expect, it } from 'vite-plus/test';
import { parseComponentMap } from '../../scripts/ci-scope.mjs';
import { validateReleaseWorkspace } from '../../scripts/release-workspace.mjs';
import type { Workspace, WorkspaceCrate } from '../../scripts/cargo-workspace.mjs';

function fixture() {
  const definitions = {
    cli: { package: 'tmt-cli', owns: ['.'], excludes: ['extensions'] },
    squad: { package: 'tmt-squad', owns: ['extensions/squad'] },
    tui: { owns: ['rust/tui'], release: false, releaseConsumers: ['squad'] },
    style: { owns: ['rust/style'], release: false, releaseConsumers: ['squad'] },
    invoke: { owns: ['rust/invoke'], release: false, releaseConsumers: ['squad'] },
    'driver-herdr': { owns: ['driver'], package: 'tmt-driver-herdr', release: false },
  };
  const crate = (
    name: string,
    dir: string,
    dependencies: string[] = [],
    hasBinary = false
  ): WorkspaceCrate => ({
    name,
    dir,
    manifest: `${dir}/Cargo.toml`,
    version: '5.0.0-dev',
    inheritsVersion: name !== 'tmt-squad' && name !== 'tmt-driver-herdr',
    dependencies,
    hasBinary,
    ...(name === 'tmt-driver-herdr' ? { dist: false } : {}),
  });
  const crates = [
    crate('tmt-cli', 'rust/cli', ['style'], true),
    crate('tmt-squad', 'extensions/squad', ['tui', 'invoke'], true),
    crate('tui', 'rust/tui', ['style']),
    crate('style', 'rust/style'),
    crate('invoke', 'rust/invoke'),
    crate('tmt-driver-herdr', 'driver', [], true),
  ];
  const workspace: Workspace = { crates, lockNames: new Set(crates.map((c) => c.name)), files: [] };
  const check = (
    definitionsOverride: Record<string, unknown> = definitions,
    workspaceOverride = workspace
  ) =>
    validateReleaseWorkspace({
      map: parseComponentMap(JSON.stringify({ components: definitionsOverride })),
      workspace: workspaceOverride,
    });
  return { definitions, crates, workspace, check };
}

describe('release workspace coverage independent of release-please', () => {
  it('accepts every reviewed direct/transitive private leaf and keeps the driver parked', () => {
    expect(fixture().check()).toEqual(['cli', 'squad']);
  });
  it.each(['style', 'invoke'] as const)(
    'reports an unregistered %s with leaf, consumer and map fix',
    (name) => {
      const f = fixture();
      expect(f.check()).toEqual(['cli', 'squad']);
      const missing = { ...f.definitions };
      delete (missing as Partial<typeof missing>)[name];
      expect(() => f.check(missing)).toThrow(
        `Workspace leaf ${name} linked by release consumer squad has no release attribution.`
      );
      expect(() => f.check(missing)).toThrow(
        `Declare a private component owning rust/${name} with releaseConsumers: ["squad"]`
      );
    }
  );
  it('rejects a registered leaf without the required consumer, with the same positive control', () => {
    const f = fixture();
    expect(f.check()).toEqual(['cli', 'squad']);
    expect(() =>
      f.check({ ...f.definitions, style: { owns: f.definitions.style.owns, release: false } })
    ).toThrow('Workspace leaf style');
  });
  it('keeps library-only leaves valid and refuses a private binary without explicit dist=false', () => {
    const f = fixture();
    expect(f.check()).toEqual(['cli', 'squad']);
    expect(() =>
      f.check(f.definitions, {
        ...f.workspace,
        crates: f.crates.map((c) =>
          c.name === 'tmt-driver-herdr' ? { ...c, dist: undefined } : c
        ),
      })
    ).toThrow('binary without dist=false');
  });
  it('fails closed on missing product/package/lock/graph evidence', () => {
    const f = fixture();
    expect(() =>
      f.check(f.definitions, {
        ...f.workspace,
        crates: f.crates.filter((c) => c.name !== 'tmt-squad'),
      })
    ).toThrow('Cargo package');
    expect(() =>
      f.check(f.definitions, { ...f.workspace, lockNames: new Set(['tmt-cli']) })
    ).toThrow('lock has no entry');
    expect(() =>
      f.check(f.definitions, {
        ...f.workspace,
        crates: f.crates.map((c) => (c.name === 'tui' ? { ...c, dependencies: ['missing'] } : c)),
      })
    ).toThrow('Missing workspace dependency');
  });
});
