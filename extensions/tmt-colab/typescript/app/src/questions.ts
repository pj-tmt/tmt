import {
  COMMENT_BYTES,
  compareCommentOrder,
  type CommentView,
  type ThreadView,
} from './thread-records.js';

export const QUESTION_LIMIT = 50;
export const REVIEW_LIMIT = 50;
export const READING_LIMIT = 50;
export const QUESTION_GROUP_LIMIT = 12;
export const QUESTION_LABEL_POINTS = 24;
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
  group: string | null;
  label: string | null;
}
export interface QuestionGroup {
  id: string;
  title: string | null;
  questions: string[];
  /** `chip`: the author opted into the compact inline form (`data-form="chip"`); absent = block. */
  form?: 'chip';
}
/** An admitted review section: the authored heading is its quote anchor. `questions` lists the
 * admitted questions inside it, in document order; answering all of them closes the section. */
export interface Review {
  id: string;
  title: string;
  agent: string;
  questions: string[];
}
/** Inert authored metadata; a person's thread record owns the read state. */
export interface Reading {
  id: string;
  title: string;
  agent: string;
  why: string;
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
  | 'question-marker-misplaced'
  | 'question-group-child-invalid'
  | 'group-id-invalid'
  | 'group-id-duplicate'
  | 'group-nested'
  | 'group-title-missing'
  | 'group-title-multiple'
  | 'group-title-empty'
  | 'group-title-too-long'
  | 'group-title-control'
  | 'group-questions-too-many'
  | 'group-empty'
  | 'group-label-too-long'
  | 'group-label-control'
  | 'group-form-invalid'
  | 'question-form-invalid'
  | 'question-form-member'
  | 'review-page-limit'
  | 'review-id-invalid'
  | 'review-id-duplicate'
  | 'review-nested'
  | 'review-heading-missing'
  | 'review-heading-multiple'
  | 'review-title-empty'
  | 'review-title-too-long'
  | 'review-title-control'
  | 'review-agent-invalid'
  | 'reading-page-limit'
  | 'reading-id-invalid'
  | 'reading-id-duplicate'
  | 'reading-nested'
  | 'reading-heading-missing'
  | 'reading-heading-multiple'
  | 'reading-title-empty'
  | 'reading-title-too-long'
  | 'reading-title-control'
  | 'reading-agent-invalid'
  | 'reading-why-invalid';
export interface QuestionFinding {
  /** Lowercase tag name, and the 1-based index among elements with that tag in document order. */
  element: string;
  ordinal: number;
  /** The authored `data-id`, or null when the element has none. */
  id: string | null;
  /** `ignored`: the block is dropped. `warning`: admitted, but something is off. */
  severity: 'ignored' | 'warning';
  code: QuestionFindingCode;
  /** All question descendants excluded by a structural group failure. */
  questionsSkipped?: number;
}
export interface QuestionInspection {
  questions: Question[];
  groups: QuestionGroup[];
  reviews: Review[];
  readings: Reading[];
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
  groupChildInvalid: boolean;
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
  ['question-group-child-invalid', (c) => c.groupChildInvalid],
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
interface GroupCandidate {
  id: string;
  headings: number;
  title: string;
  nested: boolean;
  count: number;
  holders: number;
}
/** Structural group rules report only the first failure and silence the subtree. */
const GROUP_RULES: ReadonlyArray<readonly [QuestionFindingCode, (c: GroupCandidate) => boolean]> = [
  ['group-id-invalid', (c) => !uuid.test(c.id)],
  ['group-id-duplicate', (c) => uuid.test(c.id) && c.holders > 1],
  ['group-nested', (c) => c.nested],
  ['group-title-missing', (c) => c.headings === 0],
  ['group-title-multiple', (c) => c.headings > 1],
  ['group-title-empty', (c) => c.headings === 1 && !c.title],
  ['group-title-too-long', (c) => c.headings === 1 && over(c.title, QUESTION_TITLE_POINTS)],
  ['group-title-control', (c) => c.headings === 1 && hasControl(c.title)],
  ['group-questions-too-many', (c) => c.count > QUESTION_GROUP_LIMIT],
];
interface ReviewCandidate {
  id: string;
  agent: string;
  headings: number;
  title: string;
  nested: boolean;
  holders: number;
}
/** The review section rules, in report order. A section is admitted exactly when none fails; the
 * title rules wait for exactly one heading. The Rust checker runs the same table. */
const REVIEW_RULES: ReadonlyArray<readonly [QuestionFindingCode, (c: ReviewCandidate) => boolean]> =
  [
    ['review-id-invalid', (c) => !uuid.test(c.id)],
    ['review-id-duplicate', (c) => uuid.test(c.id) && c.holders > 1],
    ['review-nested', (c) => c.nested],
    ['review-heading-missing', (c) => c.headings === 0],
    ['review-heading-multiple', (c) => c.headings > 1],
    ['review-title-empty', (c) => c.headings === 1 && !c.title],
    ['review-title-too-long', (c) => c.headings === 1 && over(c.title, QUESTION_TITLE_POINTS)],
    ['review-title-control', (c) => c.headings === 1 && hasControl(c.title)],
    ['review-agent-invalid', (c) => over(c.agent, QUESTION_AGENT_POINTS) || hasControl(c.agent)],
  ];
interface ReadingCandidate extends ReviewCandidate {
  why: string;
}
const READING_RULES: ReadonlyArray<
  readonly [QuestionFindingCode, (c: ReadingCandidate) => boolean]
> = [
  ['reading-id-invalid', (c) => !uuid.test(c.id)],
  ['reading-id-duplicate', (c) => uuid.test(c.id) && c.holders > 1],
  ['reading-nested', (c) => c.nested],
  ['reading-heading-missing', (c) => c.headings === 0],
  ['reading-heading-multiple', (c) => c.headings > 1],
  ['reading-title-empty', (c) => c.headings === 1 && !c.title],
  ['reading-title-too-long', (c) => c.headings === 1 && over(c.title, QUESTION_TITLE_POINTS)],
  ['reading-title-control', (c) => c.headings === 1 && hasControl(c.title)],
  [
    'reading-agent-invalid',
    (c) => !c.agent.trim() || over(c.agent, QUESTION_AGENT_POINTS) || hasControl(c.agent),
  ],
  [
    'reading-why-invalid',
    (c) => !c.why.trim() || over(c.why, QUESTION_TITLE_POINTS) || hasControl(c.why),
  ],
];
const LABEL_RULES: ReadonlyArray<readonly [QuestionFindingCode, (label: string) => boolean]> = [
  ['group-label-too-long', (label) => over(label, QUESTION_LABEL_POINTS)],
  ['group-label-control', hasControl],
];
/** `data-form` is opt-in and has one value: `chip`. Anything else is a warning and the block form. */
const formOf = (node: Element): 'chip' | 'invalid' | undefined =>
  !node.hasAttribute('data-form')
    ? undefined
    : node.getAttribute('data-form') === 'chip'
      ? 'chip'
      : 'invalid';
/** Every code the checks can report, for the vector coverage test. */
export const QUESTION_FINDING_CODES: readonly QuestionFindingCode[] = [
  'question-page-limit',
  ...QUESTION_RULES.map(([code]) => code),
  'question-marker-misplaced',
  ...GROUP_RULES.map(([code]) => code),
  'group-empty',
  ...LABEL_RULES.map(([code]) => code),
  'group-form-invalid',
  'question-form-invalid',
  'question-form-member',
  'review-page-limit',
  ...REVIEW_RULES.map(([code]) => code),
  'reading-page-limit',
  ...READING_RULES.map(([code]) => code),
];

/**
 * Parent-parse inert authored source. Group IDs and candidate question IDs use one fixed holder
 * snapshot after non-duplicate structural checks. Duplicate failures never retract holders.
 * Structural failures silence descendants; group-empty preserves evaluated child findings.
 */
export function inspectQuestions(source: string): QuestionInspection {
  const template = document.createElement('template');
  template.innerHTML = source;
  const root = template.content;
  const nodes = [...root.querySelectorAll('tmt-question')];
  const groupNodes = [...root.querySelectorAll('tmt-questions')];
  const allReviewNodes = [...root.querySelectorAll('tmt-review')];
  const allReadingNodes = [...root.querySelectorAll('tmt-read')];
  const readingNodes = allReadingNodes.length > READING_LIMIT ? [] : allReadingNodes;
  // Past the cap the page supplies no sections at all, and none of them holds an ID.
  const reviewNodes = allReviewNodes.length > REVIEW_LIMIT ? [] : allReviewNodes;
  const authored = (node: Element | undefined) => node?.getAttribute('data-id') ?? null;
  const idOf = (node: Element) => authored(node) ?? '';
  if (nodes.length > QUESTION_LIMIT)
    return {
      questions: [],
      groups: [],
      reviews: [],
      readings: [],
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
  const groupCandidates = new Map<Element, GroupCandidate>();
  for (const node of groupNodes) {
    const headings = [...node.children].filter((child) => child.matches('p[data-title]'));
    groupCandidates.set(node, {
      id: idOf(node),
      headings: headings.length,
      title: questionText(headings[0]?.textContent ?? ''),
      nested: !!node.parentElement?.closest('tmt-questions, tmt-question, tmt-read'),
      count: [...node.children].filter((child) => child.matches('tmt-question')).length,
      holders: 0,
    });
  }
  const nonDuplicateFailure = (node: Element) =>
    GROUP_RULES.some(
      ([code, fails]) => code !== 'group-id-duplicate' && fails(groupCandidates.get(node)!),
    );
  const holders = new Map<string, number>();
  const hold = (node: Element) => {
    const id = idOf(node);
    holders.set(id, (holders.get(id) ?? 0) + 1);
  };
  // Every group's own ID participates, including structurally invalid groups. Direct children
  // hold IDs only when non-duplicate structural rules passed; wrapped children never hold IDs.
  groupNodes.forEach(hold);
  reviewNodes.forEach(hold);
  readingNodes.forEach(hold);
  const questionHolders = new Set<Element>();
  for (const node of nodes) {
    const group = node.parentElement?.closest('tmt-questions');
    if (!group || (node.parentElement === group && !nonDuplicateFailure(group))) {
      hold(node);
      questionHolders.add(node);
    }
  }
  const groupFailures = new Map<Element, QuestionFindingCode>();
  for (const node of groupNodes) {
    const candidate = groupCandidates.get(node)!;
    candidate.holders = holders.get(candidate.id) ?? 0;
    const failed = GROUP_RULES.find(([, fails]) => fails(candidate));
    if (failed) groupFailures.set(node, failed[0]);
  }
  const suppressed = (node: Element) => {
    for (
      let group = node.parentElement?.closest('tmt-questions');
      group;
      group = group.parentElement?.closest('tmt-questions')
    ) {
      if (groupFailures.has(group)) return true;
    }
    return false;
  };
  const questions: Question[] = [];
  const admittedNodes = new Map<Element, Question>();
  const findings: QuestionFinding[] = [];
  groupNodes.forEach((node, index) => {
    const code = groupFailures.get(node);
    if (code && !suppressed(node))
      findings.push({
        element: 'tmt-questions',
        ordinal: index + 1,
        id: authored(node),
        severity: 'ignored',
        code,
        questionsSkipped: node.querySelectorAll('tmt-question').length,
      });
  });
  nodes.forEach((node, index) => {
    if (suppressed(node)) return;
    const id = idOf(node);
    const group = node.parentElement?.closest('tmt-questions') ?? null;
    const nested = !!node.parentElement?.closest('tmt-question, tmt-read');
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
      nested,
      holders: questionHolders.has(node) ? (holders.get(id) ?? 0) : 0,
      groupChildInvalid: !!group && node.parentElement !== group && !nested,
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
    if (failed.length) return;
    let label: string | null = null;
    if (group) {
      const authoredLabel = questionText(node.getAttribute('data-label') ?? '');
      const badLabel = LABEL_RULES.filter(([, fails]) => fails(authoredLabel));
      for (const [code] of badLabel)
        findings.push({
          element: 'tmt-question',
          ordinal: index + 1,
          id: authored(node),
          severity: 'warning',
          code,
        });
      label =
        authoredLabel && !badLabel.length
          ? authoredLabel
          : `Q${questions.filter((question) => question.group === idOf(group)).length + 1}`;
      // The group decides the form; a member's own attribute is ignored.
      if (node.hasAttribute('data-form'))
        findings.push({
          element: 'tmt-question',
          ordinal: index + 1,
          id: authored(node),
          severity: 'warning',
          code: 'question-form-member',
        });
    }
    const question: Question = {
      id,
      title: candidate.title,
      kind: candidate.kind as QuestionKind,
      choices: candidate.choices,
      other: candidate.other,
      agent: candidate.agent,
      group: group ? idOf(group) : null,
      label,
    };
    questions.push(question);
    admittedNodes.set(node, question);
  });
  const reviews: Review[] = [];
  if (allReviewNodes.length > REVIEW_LIMIT)
    findings.push({
      element: 'tmt-review',
      ordinal: REVIEW_LIMIT + 1,
      id: authored(allReviewNodes[REVIEW_LIMIT]),
      severity: 'ignored',
      code: 'review-page-limit',
    });
  reviewNodes.forEach((node, index) => {
    const headings = [...node.children].filter((child) => child.hasAttribute('data-title'));
    const candidate: ReviewCandidate = {
      id: idOf(node),
      agent: node.getAttribute('data-agent') ?? '',
      headings: headings.length,
      title: questionText(headings[0]?.textContent ?? ''),
      nested: !!node.parentElement?.closest('tmt-review, tmt-question, tmt-questions, tmt-read'),
      holders: holders.get(idOf(node)) ?? 0,
    };
    const failed = REVIEW_RULES.filter(([, fails]) => fails(candidate));
    for (const [code] of failed)
      findings.push({
        element: 'tmt-review',
        ordinal: index + 1,
        id: authored(node),
        severity: 'ignored',
        code,
      });
    if (failed.length) return;
    reviews.push({
      id: candidate.id,
      title: candidate.title,
      agent: candidate.agent,
      questions: [...admittedNodes]
        .filter(([question]) => node.contains(question))
        .map(([, question]) => question.id),
    });
  });
  const readings: Reading[] = [];
  if (allReadingNodes.length > READING_LIMIT)
    findings.push({
      element: 'tmt-read',
      ordinal: READING_LIMIT + 1,
      id: authored(allReadingNodes[READING_LIMIT]),
      severity: 'ignored',
      code: 'reading-page-limit',
    });
  readingNodes.forEach((node, index) => {
    const headings = [...node.children].filter((child) => child.hasAttribute('data-title'));
    const candidate: ReadingCandidate = {
      id: idOf(node),
      agent: node.getAttribute('data-agent') ?? '',
      why: node.getAttribute('data-why') ?? '',
      headings: headings.length,
      title: questionText(headings[0]?.textContent ?? ''),
      nested:
        !!node.parentElement?.closest('tmt-read, tmt-review, tmt-question, tmt-questions') ||
        !!node.querySelector('tmt-read, tmt-review, tmt-question, tmt-questions'),
      holders: holders.get(idOf(node)) ?? 0,
    };
    const failed = READING_RULES.filter(([, fails]) => fails(candidate));
    for (const [code] of failed)
      findings.push({
        element: 'tmt-read',
        ordinal: index + 1,
        id: authored(node),
        severity: 'ignored',
        code,
      });
    if (!failed.length)
      readings.push({
        id: candidate.id,
        title: candidate.title,
        agent: candidate.agent,
        why: candidate.why,
      });
  });
  const groups: QuestionGroup[] = [];
  // Preserve authored group/lone-question order. Empty is a post-child result, never structural.
  for (const node of root.querySelectorAll('tmt-questions, tmt-question')) {
    if (node.matches('tmt-questions')) {
      if (groupFailures.has(node) || suppressed(node)) continue;
      const members = questions.filter((question) => question.group === idOf(node));
      if (!members.length)
        findings.push({
          element: 'tmt-questions',
          ordinal: groupNodes.indexOf(node) + 1,
          id: authored(node),
          severity: 'ignored',
          code: 'group-empty',
        });
      else {
        const form = formOf(node);
        if (form === 'invalid')
          findings.push({
            element: 'tmt-questions',
            ordinal: groupNodes.indexOf(node) + 1,
            id: authored(node),
            severity: 'warning',
            code: 'group-form-invalid',
          });
        groups.push({
          id: idOf(node),
          title: groupCandidates.get(node)!.title,
          questions: members.map((question) => question.id),
          ...(form === 'chip' ? { form } : {}),
        });
      }
    } else {
      const question = admittedNodes.get(node);
      if (question && !question.group) {
        const form = formOf(node);
        if (form === 'invalid')
          findings.push({
            element: 'tmt-question',
            ordinal: nodes.indexOf(node) + 1,
            id: authored(node),
            severity: 'warning',
            code: 'question-form-invalid',
          });
        groups.push({
          id: question.id,
          title: null,
          questions: [question.id],
          ...(form === 'chip' ? { form } : {}),
        });
      }
    }
  }
  // Marker warnings in a structurally skipped subtree are also silent.
  for (const marker of root.querySelectorAll('[data-question], [data-choice]')) {
    if (suppressed(marker) || marker.parentElement?.matches('tmt-question')) continue;
    findings.push({
      element: marker.localName,
      ordinal: [...root.querySelectorAll(marker.localName)].indexOf(marker) + 1,
      id: authored(marker),
      severity: 'warning',
      code: 'question-marker-misplaced',
    });
  }
  return { questions, groups, reviews, readings, findings };
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

const answerPrefix = (question: Pick<Question, 'title'>) => `Answer to “${question.title}”: `;

/** The answer a comment body carries for `question`, without the `Page:` line and the recipient
 * mentions `answerComment` appends; undefined for any other comment. */
export function answerOf(question: Pick<Question, 'title'>, body: string): string | undefined {
  const prefix = answerPrefix(question);
  if (!body.startsWith(prefix)) return undefined;
  const answer = body.slice(prefix.length);
  const page = answer.lastIndexOf('\nPage: ');
  return page >= 0 ? answer.slice(0, page) : answer;
}

/** The caller supplies only verified comments associated by resolved normal quote selectors.
 * Publisher timestamps and agent replies never decide the current answer. */
export function currentAnswer(question: Question, comments: readonly CommentView[]) {
  return comments
    .filter(
      (comment) =>
        comment.deviceAuthor === true &&
        !comment.deleted &&
        comment.body.startsWith(answerPrefix(question)),
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
