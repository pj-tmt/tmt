import { text } from './strings.js';

export function relativeTime(issuedAt: number, now: number) {
  if (now === 0) return text.askJustNow;
  const minutes = Math.round((issuedAt - now) / 60000);
  if (Math.abs(minutes) < 1) return text.askJustNow;
  const format = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  if (Math.abs(minutes) < 60) return format.format(minutes, 'minute');
  const hours = Math.round(minutes / 60);
  return Math.abs(hours) < 24
    ? format.format(hours, 'hour')
    : format.format(Math.round(hours / 24), 'day');
}

/** Compact labels for conversation turns; publisher times remain display-only. */
export function compactRelativeTime(issuedAt: number, now: number) {
  const minutes = Math.round((issuedAt - now) / 60000);
  if (now === 0 || Math.abs(minutes) < 1) return 'just now';
  const distance = Math.abs(minutes);
  const [amount, unit] =
    distance < 60
      ? [distance, 'm']
      : distance < 1440
        ? [Math.round(distance / 60), 'h']
        : [Math.round(distance / 1440), 'd'];
  return minutes < 0 ? `${amount}${unit} ago` : `in ${amount}${unit}`;
}
