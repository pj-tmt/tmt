import assert from "node:assert/strict";
import { test } from "node:test";
import { isColabShortcut, isImeConfirmation } from "../src/components/colab-embed/keyboard.ts";

test("IME Enter confirms text without submitting, including Safari's ending keydown", () => {
  assert.equal(isImeConfirmation({ isComposing: false, keyCode: 13 }, false), false);
  assert.equal(isImeConfirmation({ isComposing: true, keyCode: 13 }, false), true);
  assert.equal(isImeConfirmation({ isComposing: false, keyCode: 13 }, true), true);
  assert.equal(isImeConfirmation({ isComposing: false, keyCode: 229 }, false), true);
});

test("Colab shortcut ignores editable targets, composition, repeats and other modifiers", () => {
  const key = {
    code: "KeyC",
    altKey: false,
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    repeat: false,
  };
  assert.equal(isColabShortcut(key, false), true);
  assert.equal(isColabShortcut(key, true), false);
  for (const change of [
    { repeat: true },
    { isComposing: true },
    { keyCode: 229 },
    { ctrlKey: true },
    { metaKey: true },
    { shiftKey: true },
    { altKey: true },
    { code: "KeyV" },
  ]) {
    assert.equal(isColabShortcut({ ...key, ...change }, false), false);
  }
});
