import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

// Execute the component's actual activation callbacks with controlled state.
// This covers the synchronous pending-ref window before React renders busy.
const file = ts.createSourceFile(
  "ColabEmbed.tsx",
  readFileSync(new URL("../src/components/colab-embed/ColabEmbed.tsx", import.meta.url), "utf8"),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
function clickHandler(attribute, value) {
  const handlers = [];
  function visit(node) {
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const attributes = node.attributes.properties.filter(ts.isJsxAttribute);
      const target = attributes.find((item) => item.name.getText(file) === attribute)?.initializer;
      const text =
        target && ts.isStringLiteral(target)
          ? target.text
          : target &&
              ts.isJsxExpression(target) &&
              target.expression &&
              ts.isTemplateExpression(target.expression)
            ? target.expression.head.text.trim()
            : undefined;
      if (text === value) {
        const click = attributes.find((item) => item.name.getText(file) === "onClick")?.initializer;
        assert.ok(
          click &&
            ts.isJsxExpression(click) &&
            click.expression &&
            ts.isArrowFunction(click.expression),
        );
        handlers.push(click.expression.getText(file));
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(file);
  assert.equal(handlers.length, 1, `one ${value} activation callback`);
  return handlers[0];
}
for (const [name, attribute, value, destination] of [
  ["Details", "aria-label", "Colab details", null],
  ["annotation marker", "className", "ce-marker", "next-thread"],
  ["Details thread link", "className", "ce-thread-link", "next-thread"],
]) {
  test(`${name} preserves the active thread while sending and navigates when idle`, () => {
    const source = clickHandler(attribute, value);
    for (const [busy, pending] of [
      [true, false],
      [false, true],
      [true, true],
      [false, false],
    ]) {
      let active = "sending-thread";
      const effects = [];
      const callback = runInNewContext(`(${source})`, {
        busy,
        submission: { current: { pending } },
        setup: false,
        item: { id: "next-thread", anchor: {} },
        setActive: (id) => {
          active = id;
          effects.push("active");
        },
        setSetup: () => effects.push("setup"),
        setMode: () => effects.push("mode"),
        resolve: () => ({ scrollIntoView: () => effects.push("scroll") }),
      });
      callback();
      assert.equal(
        active,
        busy || pending ? "sending-thread" : destination,
        `busy=${busy}, pending=${pending}`,
      );
      if (busy || pending)
        assert.deepEqual(effects, [], "refusal has no navigation or scroll effects");
      else assert.ok(effects.includes("active"), "idle activation remains reachable");
    }
  });
}
