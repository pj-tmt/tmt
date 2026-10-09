import { expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserCommand } from '../src/react.js';

it('keeps command bytes literal and one host feedback announcement without embedding copy behavior', () => {
  const html = renderToStaticMarkup(
    <BrowserCommand
      text={'tmt result "<id>"\n# café'}
      action={
        <button type="button" className="tmt-ui-command-copy">
          Copy
        </button>
      }
      feedback="Copied."
    />,
  );
  expect(html).toContain('&lt;id&gt;');
  expect(html).toContain('\n# café');
  expect(html.match(/role="status"/g)).toHaveLength(1);
  expect(html).toContain('class="tmt-ui-command-feedback"');
  expect(html).not.toContain('onclick');
});
