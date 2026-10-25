/** The two template marks of a page's content metadata (#5066): `template` makes the page a
 * template and `fromTemplate` records what a page was created from. Pure values: no lookup of
 * other pages, no authority. Mirrors `template_marks.rs`. */
import { keys, plain, requireValue } from './closed.js';

/** The compiled templates. Their names are reserved: no page can be marked with one, though a
 * page can be created from one. */
export const BUILT_IN_TEMPLATES = [
  'starter',
  'report',
  'review',
  'compare',
  'board',
  'gallery',
] as const;
export const TEMPLATE_DESCRIPTION_BYTES = 200;

export interface TemplateMark {
  name: string;
  description: string;
}
export interface TemplateSource {
  name: string;
  /** The template page's revision as an unsigned decimal string; a user template only. */
  revision?: string;
}

/** 1 to 40 characters of `a-z`, `0-9` and `-`. */
function validName(name: unknown): name is string {
  return typeof name === 'string' && /^[a-z0-9-]{1,40}$/.test(name);
}

export function validateTemplateMark(value: unknown): asserts value is TemplateMark {
  const mark = keys(value, ['name', 'description']);
  requireValue(
    validName(mark.name) && !(BUILT_IN_TEMPLATES as readonly string[]).includes(mark.name),
  );
  plain(mark.description, TEMPLATE_DESCRIPTION_BYTES);
  // One trimmed line: `plain` already refused control characters, so no line break remains.
  requireValue(mark.description === mark.description.trim());
}

export function validateTemplateSource(value: unknown): asserts value is TemplateSource {
  const source = keys(value, ['name'], ['revision']);
  requireValue(validName(source.name));
  if (Object.hasOwn(source, 'revision'))
    requireValue(typeof source.revision === 'string' && /^(0|[1-9][0-9]{0,14})$/.test(source.revision));
}
