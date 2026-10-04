import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import ts from "typescript";
import { english } from "../src/lang/strings.ts";

test("English chrome labels match the page registry", () => {
  const registry = readFileSync(new URL("../src/chapters/index.ts", import.meta.url), "utf8");
  const source = ts.createSourceFile("index.ts", registry, ts.ScriptTarget.Latest, true);
  const pages = source.statements
    .filter(ts.isVariableStatement)
    .flatMap((statement) => statement.declarationList.declarations)
    .find((declaration) => declaration.name.getText(source) === "pages")?.initializer;
  assert.ok(pages && ts.isArrayLiteralExpression(pages));

  function property(object, name) {
    assert.ok(ts.isObjectLiteralExpression(object));
    return object.properties.find(
      (item) => ts.isPropertyAssignment(item) && item.name.getText(source) === name,
    )?.initializer;
  }

  function stringProperty(object, name) {
    const value = property(object, name);
    assert.ok(value && ts.isStringLiteral(value), `${name} must be a string literal`);
    return value.text;
  }

  const crumbs = Object.fromEntries(
    pages.elements.map((page) => [stringProperty(page, "file"), stringProperty(page, "crumb")]),
  );
  assert.deepEqual(english.chrome.crumbs, crumbs);

  for (const page of pages.elements) {
    const status = property(page, "status");
    if (status) {
      const kind = stringProperty(status, "kind");
      assert.equal(english.chrome.status[kind], stringProperty(status, "label"), `status ${kind}`);
    }
  }
});
