import fs from 'node:fs';
import { createRequire } from 'node:module';
// Tooling-only and app-only installs use the same already locked workspace compiler.
let ts;
function compiler() {
  const local = createRequire(new URL('../../package.json', import.meta.url));
  const tooling = createRequire(
    new URL('../../../../../../typescript/package.json', import.meta.url),
  );
  let file;
  try {
    file = tooling.resolve('./node_modules/typescript');
  } catch (error) {
    if (error.code !== 'MODULE_NOT_FOUND') throw error;
    file = local.resolve('./node_modules/typescript');
  }
  return local(file);
}
import policy from './soak-policy.json' with { type: 'json' };
export { policy };
const root = new URL('../../../../../../', import.meta.url);
const scenarioFile = 'extensions/tmt-colab/typescript/app/acceptance/connection-soak.spec.ts';
const readOwner = (file) => fs.readFileSync(new URL(file, root), 'utf8');
function unwrap(node) {
  return ts.isAsExpression(node) || ts.isSatisfiesExpression(node) ? unwrap(node.expression) : node;
}
function staticObject(source, name) {
  let found;
  const visit = (node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name && node.initializer) {
      const value = unwrap(node.initializer);
      if (ts.isObjectLiteralExpression(value)) found = value;
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  if (!found) throw new Error(`Missing static object: ${name}`);
  return found;
}
function objectEntries(source, name) {
  const entries = new Map();
  for (const member of staticObject(source, name).properties) {
    if (!ts.isPropertyAssignment(member)) continue;
    const key = member.name;
    if (ts.isStringLiteral(key) || ts.isIdentifier(key)) entries.set(key.text, member.initializer);
  }
  return entries;
}
/** Only explicitly named owner files and literals can enter the public diagnostic vocabulary. */
export function loadSoakDictionary(read = readOwner) {
  ts ??= compiler();
  const messages = new Set();
  for (const owner of policy.owners) {
    const text = read(owner.file);
    const found = new Set();
    if (owner.kind === 'json') {
      const entries = JSON.parse(text)[owner.name];
      if (!Array.isArray(entries) || entries.some((entry) => typeof entry.code !== 'string'))
        throw new Error(`Non-static contract codes: ${owner.file}`);
      for (const entry of entries) found.add(entry.code);
    } else if (owner.kind === 'rust') {
      const constructor = owner.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const quoted = '"(?:[^"\\\\]|\\\\.)*"';
      const pattern = new RegExp(
        `${constructor}\\s*\\(\\s*(${quoted})\\s*,\\s*(${quoted})\\s*\\)`,
        'g',
      );
      for (const match of text.matchAll(pattern)) {
        found.add(JSON.parse(match[1]));
        found.add(JSON.parse(match[2]));
      }
    } else {
      const source = ts.createSourceFile(owner.file, text, ts.ScriptTarget.Latest, true);
      if (owner.kind === 'keys') {
        for (const key of objectEntries(source, owner.name).keys()) found.add(key);
      } else {
        const visit = (node) => {
          if (
            owner.kind === 'type' &&
            ts.isTypeAliasDeclaration(node) &&
            node.name.text === owner.name
          ) {
            if (!ts.isUnionTypeNode(node.type)) throw new Error('Non-static code union');
            for (const type of node.type.types) {
              if (!ts.isLiteralTypeNode(type) || !ts.isStringLiteral(type.literal))
                throw new Error('Non-static code member');
              found.add(type.literal.text);
            }
          }
          if (
            owner.kind === 'const' &&
            ts.isVariableDeclaration(node) &&
            node.name.getText(source) === owner.name
          ) {
            if (!node.initializer || !ts.isStringLiteral(node.initializer))
              throw new Error('Non-static code constant');
            found.add(node.initializer.text);
          }
          const argument =
            owner.kind === 'new' &&
            ts.isNewExpression(node) &&
            node.expression.getText(source) === owner.name
              ? node.arguments?.[0]
              : owner.kind === 'super' &&
                  ts.isCallExpression(node) &&
                  node.expression.kind === ts.SyntaxKind.SuperKeyword
                ? node.arguments[0]
                : undefined;
          if (argument && ts.isStringLiteral(argument)) found.add(argument.text);
          ts.forEachChild(node, visit);
        };
        visit(source);
      }
    }
    for (const name of owner.allowed) {
      if (!found.has(name)) throw new Error(`Missing static literal in ${owner.file}: ${name}`);
      messages.add(name);
    }
  }
  const source = ts.createSourceFile(
    scenarioFile,
    read(scenarioFile),
    ts.ScriptTarget.Latest,
    true,
  );
  const entries = objectEntries(source, 'SOAK_ASSERTIONS');
  const assertions = {};
  for (const id of policy.assertions) {
    const value = entries.get(id);
    if (!value || !ts.isStringLiteral(value)) throw new Error(`Non-static assertion: ${id}`);
    assertions[id] = value.text;
  }
  return {
    messages,
    codes: new Set([...messages].filter((value) => /^[A-Z][A-Z0-9_]+$/.test(value))),
    assertions,
  };
}
