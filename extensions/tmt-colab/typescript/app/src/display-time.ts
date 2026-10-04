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
