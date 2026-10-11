import { readFileSync } from 'node:fs';
import type { Question, QuestionFinding, QuestionInspection } from '../src/questions.js';

/** The shared page markup vectors (contracts/vectors/page-markup-v1.json), as the TS side reads them. */
export interface VectorQuestion extends Question {
  group: string | null;
  label: string | null;
}
export interface VectorFinding extends QuestionFinding {
  line: number;
}
export interface VectorCase {
  name: string;
  scope: 'parser' | 'checker';
  why: string;
  source: string[];
  admitted: {
    questions: VectorQuestion[];
    groups: { id: string; title: string | null; questions: string[] }[];
  };
  findings: VectorFinding[];
}
export interface Vectors {
  version: number;
  limits: Record<string, number>;
  cases: VectorCase[];
}
export const vectors = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/page-markup-v1.json', import.meta.url), 'utf8'),
) as Vectors;

/** The vector projection of one inspection: a lone question is an internal group of one. */
export function project(inspection: QuestionInspection) {
  return {
    admitted: { questions: inspection.questions, groups: inspection.groups },
    findings: sortFindings(inspection.findings),
  };
}
/** Findings are compared as a set; `line` is asserted by the Rust checker only. */
export function sortFindings<T extends QuestionFinding>(findings: readonly T[]) {
  return findings
    .map(({ line: _line, ...finding }: T & { line?: number }) => finding)
    .sort(
      (a, b) =>
        a.element.localeCompare(b.element) || a.ordinal - b.ordinal || a.code.localeCompare(b.code),
    );
}
