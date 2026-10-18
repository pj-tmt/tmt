import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { AgentChip } from '../src/components/agent-chip.js';

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
