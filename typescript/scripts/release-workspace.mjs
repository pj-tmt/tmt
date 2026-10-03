// Release-consumer coverage uses the Cargo graph and component map, not a generated config.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { ownerOf, parseComponentMap } from './ci-scope.mjs';
import { readWorkspace } from './cargo-workspace.mjs';
import { releasePolicy } from './native-release-policy.mjs';

export function validateReleaseWorkspace({ map, workspace }) {
  const { crates, lockNames } = workspace;
  const byName = new Map(crates.map((crate) => [crate.name, crate]));
  const components = new Map(map.components.map((component) => [component.name, component]));
  const workspaceOwner = ownerOf('rust/Cargo.toml', map);
  const versionOwner = (crate) =>
    crate.inheritsVersion ? workspaceOwner : ownerOf(crate.manifest, map);
  const checked = [];
  for (const component of map.components) {
    const owned = crates.filter((crate) => ownerOf(crate.manifest, map) === component.name);
    if (component.release === false) {
      if (owned.some((crate) => crate.hasBinary && crate.dist !== false))
        throw new Error(`Private component ${component.name} owns a binary without dist=false.`);
      continue;
    }
    if (!component.package)
      throw new Error(`Released component ${component.name} has no Cargo package.`);
    if (owned.filter((crate) => crate.name === component.package).length !== 1)
      throw new Error(
        `Missing or ambiguous Cargo package ${component.package} of ${component.name}.`
      );
    const { tagPrefix } = releasePolicy(component.name);
    if (tagPrefix !== (component.name === 'cli' ? 'v' : `${component.package}-v`))
      throw new Error(`Tag prefix ${tagPrefix} disagrees with Cargo package ${component.package}.`);
    for (const crate of crates.filter((crate) => versionOwner(crate) === component.name)) {
      if (!lockNames.has(crate.name))
        throw new Error(`rust/Cargo.lock has no entry for ${crate.name}.`);
    }
    // #975: preserve the opt-in boundary, transitive production graph and actionable map fix.
    if (map.components.some((leaf) => leaf.releaseConsumers.includes(component.name))) {
      const seen = new Set();
      const pending = [...owned];
      while (pending.length) {
        const crate = pending.pop();
        if (seen.has(crate.name)) continue;
        seen.add(crate.name);
        if (ownerOf(crate.manifest, map) !== component.name) {
          const leaf = components.get(ownerOf(crate.manifest, map));
          if (leaf?.release !== false || !leaf.releaseConsumers.includes(component.name))
            throw new Error(
              `Workspace leaf ${crate.name} linked by release consumer ${component.name} has no release attribution. ` +
                `Declare a private component owning ${crate.dir} with releaseConsumers: ["${component.name}"] in .github/components.json after ownership review.`
            );
        }
        for (const dependency of crate.dependencies) {
          const linked = byName.get(dependency);
          if (!linked)
            throw new Error(`Missing workspace dependency ${dependency} of ${crate.name}.`);
          pending.push(linked);
        }
      }
    }
    checked.push(component.name);
  }
  return checked;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 2) throw new Error('Usage: release-workspace.mjs');
    const map = parseComponentMap(
      readFileSync(new URL('../../.github/components.json', import.meta.url), 'utf8')
    );
    console.log(
      `Verified release workspace: ${validateReleaseWorkspace({ map, workspace: readWorkspace() }).join(', ')}.`
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
