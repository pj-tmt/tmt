import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { workspaceVersion } from '../support/workspace-version.js';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const read = (relative: string) => readFileSync(`${repository}${relative}`, 'utf8');

describe('workspaceVersion', () => {
  it('reads the version of the [workspace.package] table', () => {
    expect(
      workspaceVersion(
        [
          '[workspace]',
          'members = ["crates/a"]',
          '',
          '[workspace.package]',
          'authors = ["A", "B"]',
          'version = "5.0.0-alpha.9" # bumped by release-please',
          'edition = "2024"',
          '',
        ].join('\n')
      )
    ).toBe('5.0.0-alpha.9');
  });

  it('ignores versions of other tables, inline dependency versions and inherited versions', () => {
    expect(
      workspaceVersion(
        [
          '[package]',
          'version = "1.0.0"',
          '[workspace.dependencies]',
          'version = "2.0.0"',
          'serde = { version = "3.0.0" }',
          '[workspace.package]',
          'version.workspace = true',
          'version = "5.0.0-alpha.9"',
          '[workspace.lints]',
          'version = "4.0.0"',
        ].join('\n')
      )
    ).toBe('5.0.0-alpha.9');
  });

  it('refuses a manifest without a workspace package version instead of guessing', () => {
    for (const text of [
      '',
      '[workspace.package]\nedition = "2024"\n',
      '[package]\nversion = "1.0.0"\n',
      '[workspace.dependencies]\nversion = "1.0.0"\n',
    ]) {
      expect(() => workspaceVersion(text)).toThrow('no version in its [workspace.package] table');
    }
  });

  it('reads the key that release-please bumps for the CLI, and agrees with its manifest', () => {
    const config = JSON.parse(read('release-please-config.json')) as {
      packages: Record<
        string,
        { 'extra-files'?: { type: string; path: string; jsonpath: string }[] }
      >;
    };
    const bumped = (config.packages['.']['extra-files'] ?? []).filter(
      (file) => file.type === 'toml' && file.path === 'rust/Cargo.toml'
    );
    expect(bumped.map((file) => file.jsonpath)).toEqual(['$.workspace.package.version']);
    const manifest = JSON.parse(read('.release-please-manifest.json')) as Record<string, string>;
    expect(workspaceVersion()).toBe(manifest['.']);
  });
});
