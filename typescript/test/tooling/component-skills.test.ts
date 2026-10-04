import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vite-plus/test';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const scripts = (name: string) =>
  pathToFileURL(path.join(repositoryRoot, 'typescript', 'scripts', name)).href;

interface Component {
  name: string;
  package?: string;
  skills?: boolean;
  owns: string[];
}
const { parseComponentMap } = (await import(scripts('ci-scope.mjs'))) as {
  parseComponentMap: (text: string) => { components: Component[] };
};
const { shipsSkills } = (await import(scripts('component-skills.mjs'))) as {
  shipsSkills: (product: string, text?: string) => boolean;
};
const { productOfComponent } = (await import(scripts('native-release-policy.mjs'))) as {
  productOfComponent: (name: string) => string;
};
const text = fs.readFileSync(path.join(repositoryRoot, '.github', 'components.json'), 'utf8');
const { components } = parseComponentMap(text);
const skillsComponents = components.filter((component) => component.skills);

describe('components that ship agent skills', () => {
  it('declares at least one, and the reader agrees with the parsed map for every product', () => {
    expect(skillsComponents.length).toBeGreaterThan(0);
    for (const component of components.filter((candidate) => candidate.package)) {
      const product = productOfComponent(component.name);
      expect(shipsSkills(product), product).toBe(component.skills === true);
    }
  });

  it('refuses an unknown or ambiguous product and a malformed declaration', () => {
    expect(() => shipsSkills('nonexistent')).toThrow('Ambiguous or missing component');
    const declare = (skills: unknown, packageName: string | null = 'tmt-example') =>
      JSON.stringify({
        components: {
          example: {
            ...(packageName ? { package: packageName } : {}),
            skills,
            owns: ['extensions/tmt-example'],
          },
        },
      });
    expect(shipsSkills('example', declare(true))).toBe(true);
    expect(shipsSkills('example', declare(false))).toBe(false);
    expect(() => parseComponentMap(declare('yes'))).toThrow('skills must be boolean');
    expect(() => parseComponentMap(declare(true, null))).toThrow('needs a package');
  });

  it.each(skillsComponents.map((component) => [component.name, component] as const))(
    '%s carries a skills tree its archive includes',
    (_name, component) => {
      const root = component.owns[0];
      const skills = path.join(repositoryRoot, root, 'skills');
      const names = fs.readdirSync(skills);
      expect(names.length).toBeGreaterThan(0);
      for (const name of names)
        expect(fs.existsSync(path.join(skills, name, 'SKILL.md')), `${name}/SKILL.md`).toBe(true);
      // cargo-dist lists the directory in the package's dist include (relative to it).
      const manifest = fs.readFileSync(
        path.join(repositoryRoot, root, 'rust', component.package as string, 'Cargo.toml'),
        'utf8'
      );
      expect(manifest).toContain('"../../skills"');
    }
  );
});
