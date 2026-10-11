import { expect, it } from 'vite-plus/test';
import {
  QUESTION_AGENT_POINTS,
  QUESTION_CHOICE_POINTS,
  QUESTION_CHOICES,
  QUESTION_FINDING_CODES,
  QUESTION_LIMIT,
  QUESTION_TITLE_POINTS,
} from '../src/questions.js';
import { vectors } from './question-vectors.js';

it('the vector limits equal the named constants, so a number cannot drift', () => {
  const limits = vectors.limits;
  expect({
    pageQuestions: limits.pageQuestions,
    titlePoints: limits.titlePoints,
    choices: limits.choices,
    choicePoints: limits.choicePoints,
    agentPoints: limits.agentPoints,
  }).toEqual({
    pageQuestions: QUESTION_LIMIT,
    titlePoints: QUESTION_TITLE_POINTS,
    choices: QUESTION_CHOICES,
    choicePoints: QUESTION_CHOICE_POINTS,
    agentPoints: QUESTION_AGENT_POINTS,
  });
});

it('every reason code of the rule table has a parser case, and no case names an unknown code', () => {
  const used = new Set(
    vectors.cases
      .filter((candidate) => candidate.scope === 'parser')
      .flatMap((candidate) => candidate.findings.map((finding) => finding.code)),
  );
  expect([...used].sort()).toEqual([...QUESTION_FINDING_CODES].sort());
});

it('case names are unique and every finding sits on a real line of its source', () => {
  expect(new Set(vectors.cases.map((candidate) => candidate.name)).size).toBe(vectors.cases.length);
  for (const candidate of vectors.cases)
    for (const finding of candidate.findings)
      expect(finding.line, candidate.name).toBeLessThanOrEqual(candidate.source.length);
});
