import { cleanup } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';

// jsdom has no layout/scroll implementation. Navigation remains the real router;
// visual scrolling is checked in the browser, not claimed by these DOM tests.
beforeEach(() => vi.spyOn(window, 'scrollTo').mockImplementation(() => {}));

const dialogMethods = ['showModal', 'close'] as const;
const originalDialogMethods = dialogMethods.map((key) =>
  Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)
);
beforeEach(() => {
  // Visibility/lifetime only. Native browser tests own focus, modality and input.
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.open = true;
    },
  });
  Object.defineProperty(HTMLDialogElement.prototype, 'close', {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.open = false;
      this.dispatchEvent(new Event('close'));
    },
  });
});

afterEach(cleanup);
afterEach(() => {
  for (const [index, key] of dialogMethods.entries()) {
    const original = originalDialogMethods[index];
    if (original) Object.defineProperty(HTMLDialogElement.prototype, key, original);
    else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
  }
});
