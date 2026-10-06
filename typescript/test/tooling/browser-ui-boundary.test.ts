import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { expect, it } from 'vite-plus/test';
import { imports } from '../support/source-imports.js';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const home = path.join(root, 'design/browser-ui');
const production = path.join(home, 'src');
function forbiddenLoads(text: string): boolean {
  const source = ts.createSourceFile('module.ts', text, ts.ScriptTarget.Latest, true);
  let forbidden = false;
  const visit = (node: ts.Node): void => {
    if (
      ts.isCallExpression(node) &&
      (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        (ts.isIdentifier(node.expression) && node.expression.text === 'require')) &&
      (!node.arguments[0] || !ts.isStringLiteralLike(node.arguments[0]))
    )
      forbidden = true;
    if (
      ts.isNewExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === 'URL'
    )
      forbidden = true;
    ts.forEachChild(node, visit);
  };
  visit(source);
  return forbidden;
}
function violations(
  file: string,
  source: string,
  staticEntry: boolean,
  options: ts.CompilerOptions = {}
): string[] {
  const failures: string[] = [];
  if (forbiddenLoads(source)) failures.push('unproved runtime input');
  for (const edge of imports(source)) {
    if (!edge.startsWith('.') && !edge.startsWith('/')) {
      const resolved = ts.resolveModuleName(
        edge,
        file,
        { moduleResolution: ts.ModuleResolutionKind.Bundler, ...options },
        ts.sys
      ).resolvedModule;
      if (
        staticEntry ||
        !['react', 'react/jsx-runtime', 'lucide-react'].includes(edge) ||
        !resolved?.isExternalLibraryImport ||
        !['react', '@types/react', 'lucide-react'].includes(resolved.packageId?.name ?? '')
      )
        failures.push(edge);
      continue;
    }
    const target = ts.resolveModuleName(
      edge,
      file,
      { moduleResolution: ts.ModuleResolutionKind.Bundler, ...options },
      ts.sys
    ).resolvedModule?.resolvedFileName;
    if (
      !target ||
      !target.startsWith(`${production}/`) ||
      (staticEntry && target !== path.join(production, 'static.ts'))
    )
      failures.push(edge);
  }
  return failures;
}
function productionFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    return entry.isDirectory() ? productionFiles(file) : [file];
  });
}
it('protects production and the React-free static graph', () => {
  for (const file of productionFiles(production)) {
    if (/\.[cm]?[jt]sx?$/.test(file))
      expect(
        violations(file, readFileSync(file, 'utf8'), file === path.join(production, 'static.ts')),
        file
      ).toEqual([]);
    if (file.endsWith('.css'))
      expect(readFileSync(file, 'utf8'), file).not.toMatch(/@import\b|\burl\s*\(/i);
  }
});
it('rejects independent product/SDK/runtime/static-peer edges while admitting local and React type imports', () => {
  const file = path.join(production, 'header.tsx');
  expect(
    violations(
      file,
      "import type {ReactNode} from 'react'; import {browserUiClasses} from './static';",
      false
    )
  ).toEqual([]);
  for (const source of [
    "export * from '../../../extensions/tmt-colab/typescript/app/src/colab-header';",
    "import SDK from '@tmt/colab-client';",
    "import store from '@tanstack/react-router';",
    "import('product-' + name);",
    'require(name);',
    "new URL('../../../extensions/product/state.json', import.meta.url);",
  ])
    expect(violations(file, source, false).length).toBeGreaterThan(0);
  expect(
    violations(file, "import type {Header} from 'react';", false, {
      baseUrl: root,
      paths: { react: ['extensions/tmt-colab/typescript/app/src/colab-header.tsx'] },
    }).length
  ).toBeGreaterThan(0);
  for (const source of [
    "export * from './react';",
    "import type {ReactNode} from 'react';",
    "require('lucide-react');",
  ])
    expect(violations(path.join(production, 'static.ts'), source, true).length).toBeGreaterThan(0);
});
