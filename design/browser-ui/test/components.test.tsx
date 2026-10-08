import { describe, expect, it } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import type { MouseEvent, ReactElement } from 'react';
import type { BrowserIconActionProps } from '../src/react';
import {
  BrowserHeader,
  BrowserNotice,
  BrowserField,
  BrowserAction,
  BrowserIconAction,
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
      'aria-labelledby': 'device-name-label',
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
      'aria-labelledby': 'device-name-label',
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
  it('names one focusable contenteditable control and keeps ordered description IDs', () => {
    const html = renderToStaticMarkup(
      <BrowserField
        controlId="message"
        label="Message"
        description="Draft kept"
        descriptionId="draft"
        describedByIds={['limit draft', 'limit']}
        invalid
        renderControl={(props) => (
          <div {...props} role="textbox" aria-multiline="true" tabIndex={0} contentEditable />
        )}
      />,
    );
    expect(html).toContain('id="message-label"');
    expect(html).toContain('aria-labelledby="message-label"');
    expect(html).toContain('aria-describedby="limit draft"');
    expect(html).toContain('aria-invalid="true"');
    expect(html).toContain('role="textbox"');
    expect(html).toContain('aria-multiline="true"');
    expect(() =>
      renderToStaticMarkup(
        <BrowserField
          controlId="message"
          label="Message"
          description="Copy"
          descriptionId="message-label"
          renderControl={(props) => <textarea {...props} />}
        />,
      ),
    ).toThrow('distinct');
  });
  it('keeps icon names separate from visual tooltip copy and controlled pressed state', () => {
    const props = {
      type: 'button' as const,
      label: 'Show details',
      variant: 'text' as const,
      icon: (
        <svg>
          <title>Decorative details</title>
        </svg>
      ),
      onActivate: () => {},
    };
    for (const pressed of [undefined, false, true]) {
      const html = renderToStaticMarkup(<BrowserIconAction {...props} pressed={pressed} />);
      expect(html).toContain('aria-label="Show details"');
      expect(html).toContain('tmt-ui-icon-action-icon" aria-hidden="true"');
      expect(html).toContain('popover="manual" aria-hidden="true"');
      expect(html).toContain('tmt-ui-icon-action-tooltip-label">Show details');
      expect(html).not.toContain('aria-describedby');
      expect(html).not.toContain(' title=');
      if (pressed === undefined) expect(html).not.toContain('aria-pressed');
      else expect(html).toContain(`aria-pressed="${pressed}"`);
    }
    for (const variant of ['text', 'primary', 'destructive'] as const) {
      expect(renderToStaticMarkup(<BrowserIconAction {...props} variant={variant} />)).toContain(
        `data-variant="${variant}"`,
      );
    }
    const disabled = renderToStaticMarkup(
      <BrowserIconAction
        {...props}
        pressed
        disabled
        busy
        busyMark="◌"
        disabledReason="Access is read only"
        disabledReasonId="access-reason"
      />,
    );
    expect(disabled).toContain('disabled=""');
    expect(disabled).toContain('aria-busy="true"');
    expect(disabled).toContain('aria-describedby="access-reason"');
    expect(disabled).toContain('id="access-reason">Access is read only');
    expect(() => renderToStaticMarkup(<BrowserIconAction {...props} label=" " />)).toThrow(
      'nonempty label',
    );
    expect(() =>
      renderToStaticMarkup(<BrowserIconAction {...props} disabledReason="Blocked" />),
    ).toThrow('stable ID');
  });
  it('keeps disclosure attributes on the original button and excludes toggle semantics', () => {
    const props = {
      type: 'button' as const,
      label: 'More actions',
      variant: 'text' as const,
      icon: '+',
      onActivate: () => {},
    };
    const checkType = (_value: BrowserIconActionProps) => {};
    checkType({ ...props, expanded: false });
    checkType({ ...props, expanded: true, controls: 'host-menu' });
    // @ts-expect-error A menu trigger cannot also be a pressed toggle.
    checkType({ ...props, expanded: true, pressed: true });
    // @ts-expect-error A controlled target requires disclosure state.
    checkType({ ...props, controls: 'host-menu' });
    for (const expanded of [false, true]) {
      const html = renderToStaticMarkup(
        <BrowserIconAction {...props} expanded={expanded} controls="host-menu" />,
      );
      expect(html).toContain(`aria-expanded="${expanded}"`);
      expect(html).toContain('aria-controls="host-menu"');
      expect(html).not.toContain('aria-pressed');
    }
    const plain = renderToStaticMarkup(<BrowserIconAction {...props} />);
    expect(plain).not.toContain('aria-expanded');
    expect(plain).not.toContain('aria-controls');
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
