import { describe, expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import type { MouseEvent, ReactElement } from 'react';
import {
  BrowserHeader,
  BrowserNotice,
  BrowserField,
  BrowserAction,
  BrowserToggle,
} from '../src/react';

describe('presentation contracts', () => {
  it('retains complete header copy and explicit caption association without a caption when absent', () => {
    const withCaption = renderToStaticMarkup(
      <BrowserHeader
        productLabel="Colab"
        title="Original title"
        caption="Original author"
        captionId="author"
        brandLink={(brand) => <a href="/host">{brand}</a>}
      />,
    );
    expect(withCaption).toContain('aria-describedby="author"');
    expect(withCaption).toContain('id="author"');
    expect(withCaption).toContain('href="/host"');
    expect(withCaption.match(/<h1/g)).toHaveLength(1);
    expect(
      renderToStaticMarkup(<BrowserHeader productLabel="Remote" title="Access" />),
    ).not.toContain('tmt-ui-caption');
  });
  it('keeps tone, announcement and visible state independently supplied', () => {
    const html = renderToStaticMarkup(
      <BrowserNotice
        tone="blocked"
        announcement="none"
        mark="✗"
        stateLabel="Blocked"
        title="Not saved"
        body="Original draft retained"
      />,
    );
    expect(html).toContain('data-tone="blocked"');
    expect(html).not.toContain('role=');
    expect(html).toContain('aria-hidden="true">✗');
    expect(html).toContain('Blocked');
    expect(
      renderToStaticMarkup(
        <BrowserNotice
          tone="muted"
          announcement="status"
          mark="○"
          stateLabel="Waiting"
          title="Host title"
          body="Host copy"
        />,
      ),
    ).toContain('role="status"');
  });
  it('passes exactly the stable control association and drops absent error IDs', () => {
    const seen: unknown[] = [];
    const renderControl = (
      props: Parameters<Parameters<typeof BrowserField>[0]['renderControl']>[0],
    ) => {
      seen.push(props);
      return <input {...props} value="Original draft" readOnly />;
    };
    const html = renderToStaticMarkup(
      <BrowserField
        controlId="device-name"
        label="Name"
        description="Read only"
        descriptionId="access"
        error="Conflict"
        errorId="conflict"
        describedByIds={['limit', 'access', 'limit']}
        invalid={false}
        renderControl={renderControl}
      />,
    );
    expect(seen[0]).toEqual({
      id: 'device-name',
      className: 'tmt-ui-field-control',
      'aria-describedby': 'limit access conflict',
      'aria-invalid': false,
    });
    expect(html).toContain('for="device-name"');
    expect(html).toContain('value="Original draft"');
    renderToStaticMarkup(
      <BrowserField
        controlId="device-name"
        label="Name"
        describedByIds={['limit']}
        renderControl={renderControl}
      />,
    );
    expect(seen[1]).toEqual({
      id: 'device-name',
      className: 'tmt-ui-field-control',
      'aria-describedby': 'limit',
      'aria-invalid': undefined,
    });
    expect(() =>
      renderToStaticMarkup(
        <BrowserField
          controlId="name"
          label="Name"
          description="Copy"
          descriptionId="name"
          renderControl={renderControl}
        />,
      ),
    ).toThrow('distinct');
  });
  it('forwards the original native-button activation object and fences busy and disabled', () => {
    const calls: unknown[] = [];
    const event = { nativeEvent: { type: 'click', detail: 0 } } as MouseEvent<HTMLButtonElement>;
    const onActivate = (value: MouseEvent<HTMLButtonElement>) => calls.push(value);
    for (const state of [{}, { busy: true }, { disabled: true }]) {
      const element = BrowserAction({
        type: 'button',
        label: 'Save',
        variant: 'primary',
        onActivate,
        ...state,
      });
      const button = (element.props as { children: ReactElement[] }).children[0];
      (button.props as { onClick: (event: MouseEvent<HTMLButtonElement>) => void }).onClick(event);
    }
    expect(calls).toEqual([event]);
    expect(calls[0]).toBe(event);
    const html = renderToStaticMarkup(
      <BrowserAction
        type="submit"
        label="Save"
        variant="primary"
        busy
        busyMark="◌"
        disabledReason="Saving"
        disabledReasonId="save-reason"
        onActivate={onActivate}
      />,
    );
    const ready = renderToStaticMarkup(
      <BrowserAction
        type="submit"
        label="Save"
        variant="primary"
        busyMark="◌"
        onActivate={onActivate}
      />,
    );
    expect(ready).toContain('data-busy="false"');
    expect(ready).toContain('◌');
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain('aria-describedby="save-reason"');
    expect(html.indexOf('◌')).toBeLessThan(html.indexOf('>Save<'));
  });
  it('does not mutate controlled toggle state and keeps a fixed label and independent indicator', () => {
    const calls: unknown[] = [];
    const event = { nativeEvent: { type: 'click', detail: 0 } } as MouseEvent<HTMLButtonElement>;
    const props = {
      pressed: true,
      label: 'Enabled',
      onActivate: (value: MouseEvent<HTMLButtonElement>) => calls.push(value),
    };
    const element = BrowserToggle(props);
    (element.props as { onClick: (event: MouseEvent<HTMLButtonElement>) => void }).onClick(event);
    expect(calls[0]).toBe(event);
    expect(props.pressed).toBe(true);
    const html = renderToStaticMarkup(<BrowserToggle {...props} />);
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-hidden="true">✓');
    expect(html).toContain('Enabled');
    const disabled = BrowserToggle({ ...props, disabled: true });
    (disabled.props as { onClick: (event: MouseEvent<HTMLButtonElement>) => void }).onClick(event);
    expect(calls).toHaveLength(1);
  });
});
