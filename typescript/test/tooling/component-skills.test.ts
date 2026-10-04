import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
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
const { runPackedCommand } = (await import(scripts('packed-command.mjs'))) as {
  runPackedCommand: (
    executable: string,
    args: string[],
    options: { cwd: string; env: NodeJS.ProcessEnv; expectedStatus?: number }
  ) => string;
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
    const declare = (skills: unknown, packageName: string | null = 'tmt-squad', name = 'squad') =>
      JSON.stringify({
        components: {
          [name]: {
            ...(packageName ? { package: packageName } : {}),
            skills,
            owns: ['extensions/tmt-squad'],
          },
        },
      });
    expect(shipsSkills('squad', declare(true))).toBe(true);
    expect(shipsSkills('squad', declare(false))).toBe(false);
    expect(shipsSkills('squad', declare(undefined))).toBe(false);
    expect(shipsSkills('squad', declare(true, 'tmt-squad', 'tmt-squad'))).toBe(true);
    expect(() => shipsSkills('squad', declare('yes'))).toThrow('skills must be boolean');
    expect(() => shipsSkills('squad', declare(true, null))).toThrow('needs a package');
    const ambiguous = JSON.parse(declare(true));
    ambiguous.components['tmt-squad'] = ambiguous.components.squad;
    expect(() => shipsSkills('squad', JSON.stringify(ambiguous))).toThrow(
      'Ambiguous or missing component'
    );
    expect(() => shipsSkills('example', declare(true, 'tmt-example', 'example'))).toThrow(
      'No native publication policy for component example'
    );
  });

  it('loads skills policy from the verification image inputs with only Node', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-skills-image-'));
    try {
      const dockerfile = fs.readFileSync(
        path.join(repositoryRoot, 'typescript/test/native/artifact.Dockerfile'),
        'utf8'
      );
      const verification = dockerfile.slice(dockerfile.indexOf('WORKDIR /verification'));
      for (const line of verification.split('\n').filter((line) => line.startsWith('COPY '))) {
        const entries = line.split(/\s+/).slice(1);
        const destination = entries.pop()!;
        for (const source of entries.filter(
          (entry) => entry.startsWith('typescript/scripts/') || entry === '.github/components.json'
        )) {
          const target = path.join(
            root,
            destination,
            ...(destination.endsWith('/') ? [path.basename(source)] : [])
          );
          fs.mkdirSync(path.dirname(target), { recursive: true });
          fs.copyFileSync(path.join(repositoryRoot, source), target);
        }
      }
      const module = pathToFileURL(path.join(root, 'typescript/scripts/component-skills.mjs')).href;
      const invoke = (expectedStatus = 0) =>
        runPackedCommand(
          process.execPath,
          [
            '--input-type=module',
            '--eval',
            `import { shipsSkills } from ${JSON.stringify(module)}; console.log(shipsSkills('squad'));`,
          ],
          { cwd: root, env: { PATH: path.dirname(process.execPath), HOME: root }, expectedStatus }
        );
      expect(invoke()).toBe('true\n');
      fs.unlinkSync(path.join(root, 'typescript/scripts/native-release-policy.mjs'));
      expect(() => invoke()).toThrow('ERR_MODULE_NOT_FOUND');
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it('builds prepare verification arguments from the component policy without a skills product list', () => {
    const workflow = fs.readFileSync(
      path.join(repositoryRoot, '.github/workflows/native-release-prepare.yml'),
      'utf8'
    );
    const start = workflow.indexOf('            verification_args=(');
    const end = workflow.indexOf('          fi\n          echo "Verified $TARGET', start);
    expect(start).toBeGreaterThan(0);
    expect(end).toBeGreaterThan(start);
    const script = `node() {
      if [ "$1" = typescript/scripts/verify-native-artifact.mjs ]; then
        printf '%s\\n' "$@"
      else
        command node "$@"
      fi
    }
    ${workflow.slice(start, end)}`;
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-skills-prepare-'));
    try {
      for (const product of ['squad', 'remote', 'colab']) {
        const args = runPackedCommand(
          '/bin/bash',
          ['--noprofile', '--norc', '-euo', 'pipefail', '-c', script],
          {
            cwd: repositoryRoot,
            env: {
              PATH: path.dirname(process.execPath),
              HOME: root,
              PRODUCT: product,
              TARGET: 'fixture-target',
              RUNNER_TEMP: root,
            },
          }
        )
          .trimEnd()
          .split('\n');
        expect(args[0]).toBe('typescript/scripts/verify-native-artifact.mjs');
        const skills = args.indexOf('--skills');
        if (shipsSkills(product)) expect(args[skills + 1]).toBe(`extensions/tmt-${product}/skills`);
        else expect(skills).toBe(-1);
        expect(args[args.indexOf('--archive') + 1]).toBe(
          `target/distrib/tmt-${product}-fixture-target.tar.gz`
        );
        if (product === 'colab')
          expect(args[args.indexOf('--app-dir') + 1]).toBe(path.join(root, 'colab-app'));
        else expect(args).not.toContain('--app-dir');
      }
      expect(() =>
        runPackedCommand('/bin/bash', ['--noprofile', '--norc', '-euo', 'pipefail', '-c', script], {
          cwd: repositoryRoot,
          env: {
            PATH: path.dirname(process.execPath),
            HOME: root,
            PRODUCT: 'nonexistent',
            TARGET: 'fixture-target',
            RUNNER_TEMP: root,
          },
        })
      ).toThrow('Ambiguous or missing component');
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
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
