import fs from 'node:fs';
import ts from 'typescript';
import policy from './soak-policy.json' with { type: 'json' };

export { policy };
export interface SoakDictionary {
  codes: Set<string>;
  messages: Set<string>;
  assertions: Record<string, string>;
}
const root = new URL('../../../../../../', import.meta.url);
const scenarioFile = 'extensions/tmt-colab/typescript/app/acceptance/connection-soak.spec.ts';
const readOwner = (file: string) => fs.readFileSync(new URL(file, root), 'utf8');

function unwrap(node: ts.Expression): ts.Expression {
  return ts.isAsExpression(node) || ts.isSatisfiesExpression(node) ? unwrap(node.expression) : node;
}
function staticObject(source: ts.SourceFile, name: string) {
  let found: ts.ObjectLiteralExpression | undefined;
  const visit = (node: ts.Node) => {
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
function objectEntries(source: ts.SourceFile, name: string) {
  const entries = new Map<string, ts.Expression>();
  for (const member of staticObject(source, name).properties) {
    if (!ts.isPropertyAssignment(member)) continue;
    const key = member.name;
    if (ts.isStringLiteral(key) || ts.isIdentifier(key)) entries.set(key.text, member.initializer);
  }
  return entries;
}

/** Only explicitly named owner files and literals can enter the public diagnostic vocabulary. */
export function loadSoakDictionary(read = readOwner): SoakDictionary {
  const messages = new Set<string>();
  for (const owner of policy.owners) {
    const text = read(owner.file);
    const found = new Set<string>();
    if (owner.kind === 'json') {
      const entries = JSON.parse(text)[owner.name] as { code: unknown }[];
      if (!Array.isArray(entries) || entries.some((entry) => typeof entry.code !== 'string'))
        throw new Error(`Non-static contract codes: ${owner.file}`);
      for (const entry of entries) found.add(entry.code as string);
    } else if (owner.kind === 'rust') {
      const constructor = owner.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const quoted = '"(?:[^"\\\\]|\\\\.)*"';
      const pattern = new RegExp(
        `${constructor}\\s*\\(\\s*(${quoted})\\s*,\\s*(${quoted})\\s*\\)`,
        'g',
      );
      for (const match of text.matchAll(pattern)) {
        found.add(JSON.parse(match[1]) as string);
        found.add(JSON.parse(match[2]) as string);
      }
    } else {
      const source = ts.createSourceFile(owner.file, text, ts.ScriptTarget.Latest, true);
      if (owner.kind === 'keys') {
        for (const key of objectEntries(source, owner.name).keys()) found.add(key);
      } else {
        const visit = (node: ts.Node) => {
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
  const assertions: Record<string, string> = {};
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
