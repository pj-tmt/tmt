import {
  COMMENT_BYTES,
  compareCommentOrder,
  type CommentView,
  type ThreadView,
} from './thread-records.js';

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

export const QUESTION_AGENT_POINTS = 64;
const kinds: readonly string[] = ['single', 'multiple', 'short', 'long'];

/** The stable reason codes of the page markup check (colab-v1, page markup check). */
export type QuestionFindingCode =
  | 'question-page-limit'
  | 'question-id-invalid'
  | 'question-id-duplicate'
  | 'question-nested'
  | 'question-heading-missing'
  | 'question-heading-multiple'
  | 'question-title-empty'
  | 'question-title-too-long'
  | 'question-title-control'
  | 'question-kind-invalid'
  | 'question-choices-too-many'
  | 'question-choice-empty'
  | 'question-choice-too-long'
  | 'question-choice-control'
  | 'question-choice-duplicate'
  | 'question-choices-missing'
  | 'question-choices-unexpected'
  | 'question-other-unexpected'
  | 'question-agent-invalid'
  | 'question-marker-misplaced';
export interface QuestionFinding {
  /** Lowercase tag name, and the 1-based index among elements with that tag in document order. */
  element: string;
  ordinal: number;
  /** The authored `data-id`, or null when the element has none. */
  id: string | null;
  /** `ignored`: the block is dropped. `warning`: admitted, but something is off. */
  severity: 'ignored' | 'warning';
  code: QuestionFindingCode;
}
export interface QuestionInspection {
  questions: Question[];
  findings: QuestionFinding[];
}

interface Candidate {
  id: string;
  kind: string;
  agent: string;
  other: boolean;
  headings: number;
  title: string;
  choices: string[];
  nested: boolean;
  holders: number;
}
const over = (value: string, points: number) => [...value].length > points;
const sibling = (candidate: Candidate) => candidate.headings === 1;
const choiceKind = (candidate: Candidate) => ['single', 'multiple'].includes(candidate.kind);
/**
 * The one rule table of the markup check, in report order. A question is admitted exactly when no
 * rule fails. Title rules wait for exactly one heading and the choice-shape rules for a valid
 * kind, so one defect is not reported twice. The Rust checker runs the same table over the same
 * vectors (contracts/vectors/page-markup-v1.json).
 */
const QUESTION_RULES: ReadonlyArray<readonly [QuestionFindingCode, (c: Candidate) => boolean]> = [
  ['question-id-invalid', (c) => !uuid.test(c.id)],
  ['question-id-duplicate', (c) => uuid.test(c.id) && c.holders > 1],
  ['question-nested', (c) => c.nested],
  ['question-heading-missing', (c) => c.headings === 0],
  ['question-heading-multiple', (c) => c.headings > 1],
  ['question-title-empty', (c) => sibling(c) && !c.title],
  ['question-title-too-long', (c) => sibling(c) && over(c.title, QUESTION_TITLE_POINTS)],
  ['question-title-control', (c) => sibling(c) && hasControl(c.title)],
  ['question-kind-invalid', (c) => !kinds.includes(c.kind)],
  ['question-choices-too-many', (c) => c.choices.length > QUESTION_CHOICES],
  ['question-choice-empty', (c) => c.choices.some((choice) => !choice)],
  ['question-choice-too-long', (c) => c.choices.some((x) => over(x, QUESTION_CHOICE_POINTS))],
  ['question-choice-control', (c) => c.choices.some(hasControl)],
  ['question-choice-duplicate', (c) => new Set(c.choices).size !== c.choices.length],
  ['question-choices-missing', (c) => kinds.includes(c.kind) && choiceKind(c) && !c.choices.length],
  [
    'question-choices-unexpected',
    (c) => kinds.includes(c.kind) && !choiceKind(c) && c.choices.length > 0,
  ],
  ['question-other-unexpected', (c) => kinds.includes(c.kind) && !choiceKind(c) && c.other],
  ['question-agent-invalid', (c) => over(c.agent, QUESTION_AGENT_POINTS) || hasControl(c.agent)],
];
/** Every code the checks can report, for the vector coverage test. */
export const QUESTION_FINDING_CODES: readonly QuestionFindingCode[] = [
  'question-page-limit',
  ...QUESTION_RULES.map(([code]) => code),
  'question-marker-misplaced',
];

/**
 * Parse with the browser's own HTML tree builder and report which `tmt-question` blocks are
 * admitted and why the others are not. Author-supplied structure is inert. Only admitted source,
 * never a frame message, supplies it.
 */
export function inspectQuestions(source: string): QuestionInspection {
  const template = document.createElement('template');
  template.innerHTML = source;
  const nodes = [...template.content.querySelectorAll('tmt-question')];
  const authored = (node: Element | undefined) => node?.getAttribute('data-id') ?? null;
  if (nodes.length > QUESTION_LIMIT)
    return {
      questions: [],
      findings: [
        {
          element: 'tmt-question',
          ordinal: QUESTION_LIMIT + 1,
          id: authored(nodes[QUESTION_LIMIT]),
          severity: 'ignored',
          code: 'question-page-limit',
        },
      ],
    };
  const holders = new Map<string, number>();
  for (const node of nodes) {
    const id = node.getAttribute('data-id') ?? '';
    holders.set(id, (holders.get(id) ?? 0) + 1);
  }
  const questions: Question[] = [];
  const findings: QuestionFinding[] = [];
  nodes.forEach((node, index) => {
    const id = node.getAttribute('data-id') ?? '';
    const headings = [...node.children].filter((child) => child.hasAttribute('data-question'));
    const candidate: Candidate = {
      id,
      kind: node.getAttribute('data-kind') ?? '',
      agent: node.getAttribute('data-agent') ?? '',
      other: node.hasAttribute('data-other'),
      headings: headings.length,
      title: questionText(headings[0]?.textContent ?? ''),
      choices: [...node.children]
        .filter((child) => child.hasAttribute('data-choice'))
        .map((child) => questionText(child.textContent ?? '')),
      nested: !!node.parentElement?.closest('tmt-question'),
      holders: holders.get(id) ?? 0,
    };
    const failed = QUESTION_RULES.filter(([, fails]) => fails(candidate));
    for (const [code] of failed)
      findings.push({
        element: 'tmt-question',
        ordinal: index + 1,
        id: authored(node),
        severity: 'ignored',
        code,
      });
    if (!failed.length)
      questions.push({
        id,
        title: candidate.title,
        kind: candidate.kind as QuestionKind,
        choices: candidate.choices,
        other: candidate.other,
        agent: candidate.agent,
      });
  });
  // A marker outside a question is plain content to the browser: nothing is ignored, nothing shows.
  for (const marker of template.content.querySelectorAll('[data-question], [data-choice]')) {
    if (marker.parentElement?.matches('tmt-question')) continue;
    findings.push({
      element: marker.localName,
      ordinal: [...template.content.querySelectorAll(marker.localName)].indexOf(marker) + 1,
      id: authored(marker),
      severity: 'warning',
      code: 'question-marker-misplaced',
    });
  }
  return { questions, findings };
}

export const readQuestions = (source: string): Question[] => inspectQuestions(source).questions;

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

/** An empty later sibling cannot displace a conversation that already holds a device answer. */
export function canonicalQuestionThread<T extends Pick<ThreadView, 'ref' | 'comments'>>(
  question: Question,
  threads: readonly T[],
) {
  return [...threads].sort((a, b) => {
    const answered =
      Number(!!currentAnswer(question, b.comments)) - Number(!!currentAnswer(question, a.comments));
    if (answered) return answered;
    const left = `${a.ref.writer}:${a.ref.id}`,
      right = `${b.ref.writer}:${b.ref.id}`;
    return left < right ? -1 : left > right ? 1 : 0;
  })[0];
}
