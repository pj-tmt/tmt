import { expect, type Locator } from '@playwright/test';

/** A question line folded to its answer: the state word at the right and the answer under the title. */
export async function expectAnswered(line: Locator, answer: string) {
  await expect(line.locator('.question-line-state')).toHaveText('Answered');
  await expect(line.locator('.question-line-answer')).toContainText(answer);
}

/** History assertions hand off from the admitted fold, independent of renderer placement. */
export async function openAnsweredQuestion(line: Locator) {
  await expect(line).toBeEnabled();
  await expect(line).toHaveAttribute('aria-expanded', 'false');
  await line.press('Enter');
  await expect(line).toHaveAttribute('aria-expanded', 'true');
}
