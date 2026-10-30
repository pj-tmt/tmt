import { PROJECT_PINS } from './bootstrap.js';
import { projectNameError } from './projects.js';

/** A list the owner can store: at most `PROJECT_PINS` valid names, none repeated. */
export function validPins(names: readonly string[]): boolean {
  return (
    names.length <= PROJECT_PINS &&
    new Set(names).size === names.length &&
    names.every((name) => projectNameError(name) === undefined)
  );
}
