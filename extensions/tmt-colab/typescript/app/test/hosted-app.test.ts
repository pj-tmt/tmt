import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { HostedApp, hostedFailureCopy, HostedRequestNotice } from '../src/hosted-app.js';
import type { HostedEntryState } from '../src/hosted-entry.js';

const markup = (state: HostedEntryState) =>
  renderToStaticMarkup(createElement(HostedApp, { state, signIn() {} }));
it('a fresh hosted load has backend-neutral sign-in and a share-link next step', () => {
  const html = markup({ phase: 'sign-in', hasLink: false });
  expect(html).toMatch(/<button\b[^>]*>.*?Sign in.*?<\/button>/);
  expect(html).toContain('share link');
  expect(html).not.toContain('sample pages');
  expect(html).not.toMatch(/Firestore|Hosted|anonymous/);
  expect(html).toContain('Sign-in needed');
  expect(html).toContain('Signing in gives read access only; you cannot edit the page.');
});
it('only a verified ready view reaches the read-only reader and no writer controls appear', () => {
  const html = markup({
    phase: 'ready',
    view: { source: '<h1>Verified</h1>', title: 'Page' },
    budget: { reads: 3501, writes: 1, day: 1, state: 'warn' },
  });
  expect(html).toContain('Read-only');
  expect(html).toContain('daily reading allowance');
  expect(html).not.toMatch(/>Save</);
  expect(html).not.toMatch(/>Ask</);
  expect(html).not.toContain('source-toggle');
});
it.each(['loading', 'recovering', 'paused', 'link-needed'] as const)(
  'the %s state does not label retained content Live',
  (phase) => {
    const html = markup({ phase });
    expect(html).not.toContain('>Live<');
    expect(html).not.toContain('frame-host');
  },
);
it('later epochs use the fixed re-share copy without exposing provider diagnostics', () => {
  const failure = {
    code: 'COLAB_HOSTED_RESHARE_REQUIRED',
    message: '/private/secret-provider-detail',
  };
  expect(hostedFailureCopy(failure).body).toBe(
    'This page changed its sharing since this link was made. Ask the owner to share it again.',
  );
  expect(markup({ phase: 'failed', failure })).not.toContain(failure.message);
});
it('budget exhaustion shows the supplied reset and leaves no automatic retry action', () => {
  const failure = {
    code: 'REMOTE_BUDGET_EXHAUSTED',
    resetAtMs: Date.parse('2026-11-02T08:00:00Z'),
    retryAfterMs: 1000,
  };
  const html = markup({ phase: 'failed', failure });
  expect(html).toContain('The allowance resets ');
  expect(html).not.toContain('2026-11-02T08:00:00.000Z');
  expect(html).toContain('Limit reached');
  expect(html).toContain('role="status"');
  expect(html).not.toContain('failed');
  expect(html).not.toMatch(/Firestore|Hosted/);
  expect(html).not.toContain('<button');
});
it('an expired cloud copy and a Rules refusal have distinct bounded explanations', () => {
  expect(hostedFailureCopy({ code: 'COLAB_HOSTED_NOT_IN_CLOUD' }).title).toBe(
    'Page not in the cloud',
  );
  expect(hostedFailureCopy({ code: 'REMOTE_SCOPE_DENIED' }).body).toContain('current share link');
  const html = markup({ phase: 'failed', failure: { code: 'REMOTE_RATE_LIMITED' } });
  expect(html).toContain('Too many requests');
  expect(html).toContain('Reading is paused for a moment. Try again in a few minutes.');
  expect(html).toContain('Paused');
  expect(html).toContain('role="status"');
  expect(html).not.toContain('failed');
  expect(html).not.toMatch(/Firestore|Hosted/);
});

it.each([
  ['2026-11-02T00:00:00Z', 'Asia/Tokyo', 'The allowance resets at 17:00.'],
  ['2026-11-01T14:59:00Z', 'Asia/Tokyo', 'The allowance resets on Nov 2 at 17:00.'],
  ['2026-11-01T23:30:00Z', 'Asia/Tokyo', 'The allowance resets at 17:00.'],
  ['2026-11-02T07:30:00Z', 'America/Los_Angeles', 'The allowance resets on Nov 2 at 00:00.'],
])('formats the reset by the injected local day and minute (%s, %s)', (now, timeZone, expected) => {
  expect(
    hostedFailureCopy(
      {
        code: 'REMOTE_BUDGET_EXHAUSTED',
        resetAtMs: Date.parse('2026-11-02T08:00:00Z'),
      },
      { nowMs: Date.parse(now), timeZone },
    ).reset,
  ).toBe(expected);
});
it.each(['REMOTE_SCOPE_DENIED', 'REMOTE_STATE_UNAVAILABLE'])(
  'keeps the failure tone for %s',
  (code) => {
    const html = markup({ phase: 'failed', failure: { code } });
    expect(html).toContain('failed');
    expect(html).toContain('role="alert"');
  },
);

it.each([
  [
    'viewer',
    'Read-only',
    'Sign in to read the shared page. Signing in gives read access only; you cannot edit the page.',
  ],
  ['commenter', 'Can comment', 'Sign in to read this page and send comments.'],
  [
    'editor',
    'Can edit',
    'Sign in to read this page and send comments or edit requests. The owner applies edits while Colab is running.',
  ],
] as const)('uses the %s hint only for sign-in presentation', (role, mark, copy) => {
  const html = markup({ phase: 'sign-in', hasLink: true, role });
  expect(html).toContain(mark);
  expect(html).toContain(copy);
  expect(html).not.toContain('source-toggle');
  if (role !== 'viewer') expect(html).not.toContain('you cannot edit the page');
});

it.each([
  [
    'sent',
    'Edit request sent. It appears on the page when the owner&#x27;s Colab applies it.',
    'muted',
  ],
  ['refused', 'Edit request not sent. Ask the owner for a current link.', 'blocked'],
  [
    'unconfirmed',
    'Couldn&#x27;t confirm the edit request. It may still reach the owner; check the page before sending it again.',
    'waiting',
  ],
] as const)('keeps the %s request verdict distinct', (outcome, message, tone) => {
  const html = renderToStaticMarkup(createElement(HostedRequestNotice, { outcome }));
  expect(html).toContain(message);
  expect(html).toContain(`data-tone="${tone}"`);
  if (outcome === 'unconfirmed') {
    expect(html).toContain('Unconfirmed');
    expect(html).not.toContain('not sent');
    expect(html).not.toContain('Edit request sent');
  }
});

it.each(['viewer', 'commenter', 'editor'] as const)(
  'keeps the ready %s role mark separate from author admission',
  (role) => {
    const html = markup({
      phase: 'ready',
      role,
      edit: async () => ({ request: 'sent' }),
      view: { source: '<main>admitted</main>', title: 'Page' },
      budget: { day: 1, reads: 10, writes: 0, state: 'ok' },
    });
    expect(html).toContain(
      role === 'editor' ? 'Can edit' : role === 'commenter' ? 'Can comment' : 'Read-only',
    );
    expect(html.includes('source-toggle')).toBe(role === 'editor');
    expect(html).not.toContain('>Ask<');
  },
);

it('never labels definitive writer occupancy sent, refused-before-effect or unknown', () => {
  const html = renderToStaticMarkup(
    createElement(HostedRequestNotice, { outcome: 'writer-in-use' }),
  );
  expect(html).toContain('data-tone="blocked"');
  expect(html).not.toContain('>Sent<');
  expect(html).not.toContain('>Not sent<');
  expect(html).not.toContain('>Unconfirmed<');
  expect(html).toContain('[Pending Colab/UX writer-in-use copy]');
});
