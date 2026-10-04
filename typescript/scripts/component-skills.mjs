import { readFileSync } from 'node:fs';
import { parseComponentMap } from './ci-scope.mjs';
import { productOfComponent } from './native-release-policy.mjs';

const COMPONENT_MAP = new URL('../../.github/components.json', import.meta.url);

/**
 * Whether a native product's archive carries an agent-skills tree: the `skills` field of its
 * component in .github/components.json, the one declaration the archive policy, the verifier
 * and the tooling tests read. It needs only Node, so the verification-only image can run it.
 * Shared map validation and native publication policy own declaration and product identity.
 */
export function shipsSkills(product, text = readFileSync(COMPONENT_MAP, 'utf8')) {
  const { components } = parseComponentMap(text);
  const matches = components.filter(
    (component) => component.package && productOfComponent(component.name) === product
  );
  if (matches.length !== 1) throw new Error(`Ambiguous or missing component for ${product}.`);
  return matches[0].skills === true;
}
