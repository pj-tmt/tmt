import { PROJECT_PINS, type ProjectPins } from './bootstrap.js';
import { projectNameError } from './projects.js';

/** A list the owner can store: at most `PROJECT_PINS` valid names, none repeated. */
export function validPins(names: readonly string[]): boolean {
  return (
    names.length <= PROJECT_PINS &&
    new Set(names).size === names.length &&
    names.every((name) => projectNameError(name) === undefined)
  );
}

/**
 * The projects the side menu lists, in order: the owner's pins that the space still has. A source
 * with no pins (hosted and detached ones) lists every project, as the menu did before pins.
 */
export function menuProjects(
  projects: readonly string[],
  pins: ProjectPins | undefined,
): { names: readonly string[]; automatic: boolean } {
  if (!pins) return { names: projects, automatic: false };
  const known = new Set(projects);
  return { names: pins.names.filter((name) => known.has(name)), automatic: pins.automatic };
}

/** The list with `name` pinned (last, or where it already is) or unpinned. */
export function withPin(list: readonly string[], name: string, pinned: boolean): string[] {
  if (!pinned) return list.filter((held) => held !== name);
  return list.includes(name) ? [...list] : [...list, name];
}

/** The list with `name` one place earlier (`-1`) or later (`1`); at an end it stays. */
export function movedPin(list: readonly string[], name: string, by: -1 | 1): string[] {
  const from = list.indexOf(name);
  const to = from + by;
  if (from < 0 || to < 0 || to >= list.length) return [...list];
  const next = [...list];
  [next[from], next[to]] = [next[to], next[from]];
  return next;
}
