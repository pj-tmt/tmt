import { MentionChip } from './mention-chip.js';
/** Cosmetic tokens only; sender labels and message text grant no recipient authority. */
export function MessageText({ value, names = [] }: { value: string; names?: readonly string[] }) {
  const labels = [...new Set(names.filter(Boolean))]
    .sort((a, b) => b.length - a.length)
    .map((name) => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const known = labels.length ? `(?:${labels.join('|')})(?=$|[^\\p{L}\\p{N}_-])|` : '';
  const token = `${known}[\\p{L}\\p{N}_](?:[\\p{L}\\p{N}_.-]*[\\p{L}\\p{N}_-])?`;
  return value
    .split(new RegExp(`(?<![A-Za-z0-9_.+\\-@＠])([@＠](?:${token}))`, 'u'))
    .map((part, index) => (index % 2 === 1 ? <MentionChip token={part} key={index} /> : part));
}
