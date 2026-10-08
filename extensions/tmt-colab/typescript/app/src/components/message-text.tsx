/** Cosmetic tokens only; sender labels and message text grant no recipient authority. */
export function MessageText({ value, names = [] }: { value: string; names?: readonly string[] }) {
  const labels = [...new Set(names.filter(Boolean))]
    .sort((a, b) => b.length - a.length)
    .map((name) => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const token = labels.length
    ? `(?:${labels.join('|')})(?=$|[^\\p{L}\\p{N}_])|[\\p{L}\\p{N}_][\\p{L}\\p{N}_.-]*`
    : '[\\p{L}\\p{N}_][\\p{L}\\p{N}_.-]*';
  return value.split(new RegExp(`(?<![\\p{L}\\p{N}_])(@(?:${token}))`, 'u')).map((part, index) =>
    index % 2 === 1 ? (
      <span className="message-mention" key={index}>
        {part}
      </span>
    ) : (
      part
    ),
  );
}
