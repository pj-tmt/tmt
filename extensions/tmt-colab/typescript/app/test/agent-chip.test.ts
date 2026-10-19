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
  expect(markup).not.toContain('agent-chip-spinner');
  expect(markup).not.toContain('data-activity');
});

it('replaces the icon with a labelled spinner only while the agent is working', () => {
  const working = render({ name: 'Studio agent', activity: 'working' });
  expect(working).toContain('agent-chip-spinner');
  expect(working).toContain('aria-label="working"');
  expect(working).toContain('data-activity="working"');
  expect(working).not.toContain('lucide-bot');
  for (const activity of ['idle', 'unknown', 'ended'] as const) {
    const markup = render({ name: 'Studio agent', activity });
    expect(markup, activity).toContain('lucide-bot');
    expect(markup, activity).not.toContain('agent-chip-spinner');
  }
});

it('keeps the machine and the driver as muted suffixes and the state as data', () => {
  const markup = render({
    name: '@Agent 1',
    machine: 'My machine',
    driver: 'claude',
    state: 'offline',
    variant: 'chip',
    title: 'description',
  });
  expect(markup).toContain('data-variant="chip"');
  expect(markup).toContain('data-state="offline"');
  expect(markup).toContain('title="description"');
  expect(markup).toContain('<span class="agent-chip-machine"> · My machine</span>');
  expect(markup).toContain('<span class="agent-chip-driver"> · claude</span>');
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
        createElement(AgentChip, { name: 'Atlas', driver: 'codex' }),
        createElement(AgentList, { names: ['Atlas', 'Other'] }),
        agentSentence('Waiting for {agent}', 'Atlas'),
      ),
    ),
  );
  expect(markup.match(/aria-label="working"/g)).toHaveLength(4);
  expect(markup.match(/lucide-bot/g)).toHaveLength(1);
});

it('a page observation never animates stale, stopped, missing or ambiguous agents', () => {
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
    expect(markup).not.toContain('agent-chip-spinner');
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
  expect(markup).toContain('aria-label="working"');
});
