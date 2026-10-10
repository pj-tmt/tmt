import { COMMENT_BYTES, compareCommentOrder, type CommentView } from './thread-records.js';

export const QUESTION_LIMIT = 50;
export const QUESTION_TITLE_POINTS = 200;
export const QUESTION_CHOICES = 20;
export const QUESTION_CHOICE_POINTS = 200;
export const QUESTION_SHORT_POINTS = 200;
export const QUESTION_LONG_BYTES = 8192;
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export type QuestionKind = 'single' | 'multiple' | 'short' | 'long';
export interface Question {
  id: string;
  title: string;
  kind: QuestionKind;
  choices: string[];
  other: boolean;
  agent: string;
}
const bytes = (value: string) => new TextEncoder().encode(value).length;
export const questionText = (value: string) => value.replace(/\s+/gu, ' ').trim();
const hasControl = (value: string) =>
  [...value].some(
    (character) => character.codePointAt(0)! < 32 || character === String.fromCharCode(127),
  );
const bounded = (value: string, points: number) =>
  !!value && [...value].length <= points && !hasControl(value);

/** Author-supplied structure is inert. Only admitted source, never a frame message, supplies it. */
export function readQuestions(source: string): Question[] {
  const template = document.createElement('template');
  template.innerHTML = source;
  const nodes = [...template.content.querySelectorAll('tmt-question')];
  if (nodes.length > QUESTION_LIMIT) return [];
  const questions: Question[] = [];
  for (const node of nodes) {
    const id = node.getAttribute('data-id') ?? '';
    const kind = node.getAttribute('data-kind') ?? '';
    const headings = [...node.children].filter((child) => child.hasAttribute('data-question'));
    const title = questionText(headings[0]?.textContent ?? '');
    const choices = [...node.children]
      .filter((child) => child.hasAttribute('data-choice'))
      .map((child) => questionText(child.textContent ?? ''));
    const agent = node.getAttribute('data-agent') ?? '';
    const other = node.hasAttribute('data-other');
    if (
      !uuid.test(id) ||
      nodes.filter((other) => other.getAttribute('data-id') === id).length !== 1 ||
      node.parentElement?.closest('tmt-question') ||
      headings.length !== 1 ||
      !bounded(title, QUESTION_TITLE_POINTS) ||
      !['single', 'multiple', 'short', 'long'].includes(kind) ||
      choices.length > QUESTION_CHOICES ||
      choices.some((choice) => !bounded(choice, QUESTION_CHOICE_POINTS)) ||
      new Set(choices).size !== choices.length ||
      [...agent].length > 64 ||
      hasControl(agent) ||
      (['single', 'multiple'].includes(kind) ? !choices.length : choices.length || other)
    )
      continue;
    questions.push({ id, title, kind: kind as QuestionKind, choices, other, agent });
  }
  return questions;
}

export function answerComment(
  question: Question,
  selected: readonly string[],
  value: string,
  other: boolean,
  pageTitle: string,
  recipient: string,
): string | undefined {
  const typed = value.trim();
  if (question.kind === 'single' || question.kind === 'multiple') {
    if (
      selected.some((choice) => !question.choices.includes(choice)) ||
      new Set(selected).size !== selected.length ||
      (other && (!question.other || !typed)) ||
      (!other && !selected.length) ||
      (question.kind === 'single' && selected.length + Number(other) !== 1)
    )
      return;
  } else if (!typed || selected.length || other) return;
  if (
    bytes(typed) > QUESTION_LONG_BYTES ||
    (question.kind === 'short' && [...typed].length > QUESTION_SHORT_POINTS)
  )
    return;
  const answer =
    question.kind === 'single' || question.kind === 'multiple'
      ? [
          ...question.choices.filter((choice) => selected.includes(choice)),
          ...(other ? [`Other: ${typed}`] : []),
        ].join('; ')
      : typed;
  const comment = `Answer to “${question.title}”: ${answer}\nPage: ${pageTitle}${recipient.trim() ? `\n${recipient.trim()}` : ''}`;
  return bytes(comment) <= COMMENT_BYTES ? comment : undefined;
}

/** The caller supplies only verified comments associated by resolved normal quote selectors.
 * Publisher timestamps and agent replies never decide the current answer. */
export function currentAnswer(question: Question, comments: readonly CommentView[]) {
  return comments
    .filter(
      (comment) =>
        comment.deviceAuthor === true &&
        !comment.deleted &&
        comment.body.startsWith(`Answer to “${question.title}”: `),
    )
    .sort((a, b) =>
      compareCommentOrder(
        { sequence: Number(a.sequence ?? '0'), writer: a.ref.writer, id: a.ref.id },
        { sequence: Number(b.sequence ?? '0'), writer: b.ref.writer, id: b.ref.id },
      ),
    )
    .at(-1);
}
