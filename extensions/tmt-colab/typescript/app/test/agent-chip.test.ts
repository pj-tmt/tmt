import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { AgentChip, AgentList, agentSentence } from '../src/components/agent-chip.js';
import { AgentActivityProvider } from '../src/components/agent-activity.js';

const render = (props: Parameters<typeof AgentChip>[0]) =>
  renderToStaticMarkup(createElement(AgentChip, props));

it('shows the normal agent icon and the name, inline by default', () => {
  const markup = render({ name: 'Studio agent' });
  expect(markup).toContain('class="agent-chip"');
  expect(markup).toContain('data-variant="inline"');
  expect(markup).toContain('lucide-bot');
  expect(markup).toContain('<span class="agent-chip-name">Studio agent</span>');
  expect(markup).not.toContain('data-working');
  expect(markup).not.toContain('data-activity');
});

it('marks a working agent with a static dot and its words, never a spinner', () => {
  const working = render({ name: 'Studio agent', activity: 'working' });
  expect(working).toContain('lucide-bot');
  expect(working).toContain('data-working');
  expect(working).toContain('data-activity="working"');
  expect(working).toContain('title="Studio agent, working"');
  expect(working).toContain(
    '<span class="agent-chip-state tmt-ui-visually-hidden">, working</span>',
  );
  expect(working).not.toContain('agent-chip-spinner');
  expect(working).not.toContain('role="img"');
  // A chip that already has a description keeps it and gains the state.
  expect(render({ name: '@Atlas', activity: 'working', title: 'Atlas on Studio' })).toContain(
    'title="Atlas on Studio, working"',
  );
  for (const activity of ['idle', 'unknown', 'ended'] as const) {
    const markup = render({ name: 'Studio agent', activity });
    expect(markup, activity).toContain('lucide-bot');
    expect(markup, activity).not.toContain('data-working');
    expect(markup, activity).not.toContain('agent-chip-state');
    expect(markup, activity).not.toContain('title=');
  }
});

it('leaves the idle icon out beside an avatar but keeps the working dot', () => {
  const idle = render({ name: 'Studio agent', mark: false });
  expect(idle).not.toContain('lucide-bot');
  expect(idle).not.toContain('agent-chip-mark');
  expect(idle).toContain('<span class="agent-chip-name">Studio agent</span>');
  const working = render({ name: 'Studio agent', mark: false, activity: 'working' });
  expect(working).toContain('lucide-bot');
  expect(working).toContain('data-working');
  expect(working).toContain('agent-chip-state');
  expect(
    renderToStaticMarkup(createElement(AgentList, { names: ['alpha', 'beta'], mark: false })),
  ).not.toContain('lucide-bot');
});

it('keeps the machine as a muted suffix and the state as data', () => {
  const markup = render({
    name: '@Agent 1',
    machine: 'My machine',
    state: 'offline',
    variant: 'chip',
    title: 'description',
  });
  expect(markup).toContain('data-variant="chip"');
  expect(markup).toContain('data-state="offline"');
  expect(markup).toContain('title="description"');
  expect(markup).toContain('<span class="agent-chip-machine"> · My machine</span>');
  expect(markup).not.toContain('agent-chip-driver');
});

it('renders a trailing control after the name', () => {
  const markup = renderToStaticMarkup(
    createElement(
      AgentChip,
      { name: '@Agent 1', variant: 'chip' },
      createElement('button', { type: 'button' }, 'x'),
    ),
  );
  expect(markup.indexOf('agent-chip-name')).toBeLessThan(markup.indexOf('<button'));
});

it('puts inline chips where a sentence names its agents', () => {
  const sentence = renderToStaticMarkup(
    createElement('p', null, agentSentence('Waiting for {agent}', 'Atlas')),
  );
  expect(sentence).toMatch(
    /^<p>Waiting for <span class="agent-chip"[^>]*>.*Atlas<\/span><\/span><\/p>$/,
  );
  const list = renderToStaticMarkup(
    createElement('p', null, createElement(AgentList, { names: ['@a', '@b'] })),
  );
  expect(list.match(/class="agent-chip"/g)).toHaveLength(2);
  expect(list).toContain('</span>, <span class="agent-chip"');
  expect(agentSentence('no agent here')).toHaveLength(1);
});

it('all shared chip forms reuse the page activity observation without another read', () => {
  const observation = {
    active: true,
    receivedAt: 0,
    intervalMs: 60_000,
    entries: [],
    agents: [
      {
        machine: 'machine',
        machineName: 'This machine',
        agent: 'atlas',
        name: 'Atlas',
        state: 'working' as const,
      },
    ],
  };
  const markup = renderToStaticMarkup(
    createElement(
      AgentActivityProvider,
      { observation },
      createElement(
        'div',
        null,
        createElement(AgentChip, { name: '@Atlas', variant: 'chip' }),
        createElement(AgentChip, { name: 'Atlas' }),
        createElement(AgentList, { names: ['Atlas', 'Other'] }),
        agentSentence('Waiting for {agent}', 'Atlas'),
      ),
    ),
  );
  expect(markup.match(/data-working/g)).toHaveLength(4);
  expect(markup.match(/, working<\/span>/g)).toHaveLength(4);
  expect(markup.match(/lucide-bot/g)).toHaveLength(5);
});

it('a page observation never marks stale, stopped, missing or ambiguous agents as working', () => {
  const row = {
    machine: 'machine',
    machineName: 'Owner',
    agent: 'atlas',
    name: 'Atlas',
    state: 'working' as const,
  };
  const fresh = { active: true, receivedAt: 0, intervalMs: 60_000, entries: [], agents: [row] };
  for (const observation of [
    undefined,
    { ...fresh, active: false },
    { ...fresh, receivedAt: -120_000 },
    { ...fresh, agents: [] },
    { ...fresh, agents: [row, { ...row, machine: 'other', machineName: 'Other' }] },
    ...(['idle', 'unknown', 'ended'] as const).map((state) => ({
      ...fresh,
      agents: [{ ...row, state }],
    })),
  ]) {
    const markup = renderToStaticMarkup(
      createElement(
        AgentActivityProvider,
        { observation },
        createElement(AgentChip, { name: 'Atlas' }),
      ),
    );
    expect(markup).toContain('lucide-bot');
    expect(markup).not.toContain('data-working');
  }
  const markup = renderToStaticMarkup(
    createElement(
      AgentActivityProvider,
      {
        observation: {
          ...fresh,
          receivedAt: -119_999,
          agents: [row, { ...row, machine: 'other', machineName: 'Other', state: 'idle' }],
        },
      },
      createElement(AgentChip, { name: 'Atlas', machine: 'Owner' }),
    ),
  );
  expect(markup).toContain('data-working');
});
