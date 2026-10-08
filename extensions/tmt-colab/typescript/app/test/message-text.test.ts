import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { MessageText } from '../src/components/message-text.js';

it('keeps message bytes inert while marking admitted display names, punctuation and Unicode mentions', () => {
  const markup = renderToStaticMarkup(
    createElement(MessageText, {
      value: '@Agent (east), <script>inert</script>\n@路線-agent，replied. @someone',
      names: ['Agent (east)', '路線-agent'],
    }),
  );
  expect(markup).toBe(
    '<span class="message-mention">@Agent (east)</span>, &lt;script&gt;inert&lt;/script&gt;\n<span class="message-mention">@路線-agent</span>，replied. @someone',
  );
});

it('matches a complete longest display name without highlighting an email address', () => {
  const markup = renderToStaticMarkup(
    createElement(MessageText, {
      value: 'mail@example.com @Agent 1. @Agent 10 @Agentish @other-agent',
      names: ['Agent', 'Agent 1', 'Agent 10'],
    }),
  );
  expect(markup).toBe(
    'mail@example.com <span class="message-mention">@Agent 1</span>. <span class="message-mention">@Agent 10</span> @Agentish @other-agent',
  );
});

it('renders unbound mentions as escaped plain text when no names are supplied', () => {
  for (const names of [undefined, [], ['']]) {
    expect(
      renderToStaticMarkup(createElement(MessageText, { value: '@someone <b>plain</b>', names })),
    ).toBe('@someone &lt;b&gt;plain&lt;/b&gt;');
  }
});
