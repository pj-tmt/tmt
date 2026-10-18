import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { MessageText } from '../src/components/message-text.js';
import { AgentPresentation } from '../src/agent-directory.js';
import type { AgentDestination } from '../src/live-ask.js';

function render(value: string, names?: readonly string[]) {
  const markup = renderToStaticMarkup(createElement(MessageText, { value, names }));
  const tokens = [...markup.matchAll(/class="agent-chip-name">([^<]+)<\/span>/g)].map(
    (match) => match[1],
  );
  return { markup, tokens };
}
it('keeps message bytes inert while marking known and unknown cosmetic mentions', () => {
  const { markup, tokens } = render(
    '@Agent (east), <script>inert</script>\n@路線-agent，replied. @someone',
    ['Agent (east)', '路線-agent'],
  );
  expect(tokens).toEqual(['Agent (east)', '路線-agent', 'someone']);
  expect(markup).toContain('&lt;script&gt;inert&lt;/script&gt;');
  expect(markup).not.toContain('<script>');
  expect(markup.match(/class="agent-chip"/g)).toHaveLength(3);
});
it('uses longest names, skips email addresses and retains sentence punctuation', () => {
  const { tokens, markup } = render('mail@example.com @Agent 1. @Agent 10 @Agentish @other-agent', [
    'Agent',
    'Agent 1',
    'Agent 10',
  ]);
  expect(tokens).toEqual(['Agent 1', 'Agent 10', 'Agentish', 'other-agent']);
  expect(markup).toContain('mail@example.com ');
  expect(markup).toContain('</span>. ');
});
it('shows any mention as an agent chip without requiring directory names or granting routing', () => {
  for (const names of [undefined, [], ['']]) {
    const { markup, tokens } = render('@someone <b>plain</b> ＠路線', names);
    expect(tokens).toEqual(['someone', '路線']);
    expect(markup).toContain('class="agent-chip"');
    expect(markup).toContain('&lt;b&gt;plain&lt;/b&gt;');
    expect(markup).not.toContain('button');
  }
});
it('keeps a directory name with a space whole when the caller names no agent', () => {
  const directory = [{ agentName: 'Agent 1' }, { agentName: 'Agent' }] as AgentDestination[];
  const markup = renderToStaticMarkup(
    createElement(
      AgentPresentation.Provider,
      { value: directory },
      createElement(MessageText, { value: 'Page: Notes\n@Agent 1 and @Agent.' }),
    ),
  );
  const tokens = [...markup.matchAll(/class="agent-chip-name">([^<]+)<\/span>/g)].map(
    (match) => match[1],
  );
  expect(tokens).toEqual(['Agent 1', 'Agent']);
});
