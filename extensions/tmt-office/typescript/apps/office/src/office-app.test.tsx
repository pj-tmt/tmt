import { createMemoryHistory } from '@tanstack/react-router';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { OfficeApp } from './office-app.js';
import { createOfficeRouter } from './router.js';

function renderOffice(path = '/') {
  const router = createOfficeRouter(createMemoryHistory({ initialEntries: [path] }));
  return { ...render(<OfficeApp router={router} />), router };
}

describe('Office foundation', () => {
  it('shows an honest disconnected state rather than fabricated agents or a login action', async () => {
    renderOffice();
    await screen.findByRole('heading', { name: 'No world connected' });
    expect(screen.getByText('No cloud connection · No agents connected')).toBeDefined();
    expect(screen.queryByRole('button', { name: /sign in|connect|create world/i })).toBeNull();
  });

  it('navigates through the real router and preserves local presentation state', async () => {
    const user = userEvent.setup();
    const { router } = renderOffice();
    await screen.findByRole('heading', { name: 'No world connected' });
    await user.click(screen.getByRole('button', { name: 'Preview details' }));
    expect(
      screen.getByRole('button', { name: 'Preview details' }).getAttribute('aria-expanded')
    ).toBe('true');
    await user.click(within(screen.getByRole('navigation')).getByRole('link', { name: 'Setup' }));
    await screen.findByRole('heading', { name: 'Start small. Stay in control.' });
    expect(router.state.location.pathname).toBe('/setup');
    expect(screen.getByText(/This shell does not create a world/)).toBeDefined();
    await user.click(within(screen.getByRole('navigation')).getByRole('link', { name: 'Office' }));
    await screen.findByRole('heading', { name: 'No world connected' });
    expect(router.state.location.pathname).toBe('/');
  });

  it('supports direct setup links and unknown-route recovery', async () => {
    const direct = renderOffice('/setup');
    await screen.findByRole('heading', { name: 'A private world' });
    direct.unmount();
    const user = userEvent.setup();
    const { router } = renderOffice('/missing');
    await screen.findByRole('heading', { name: 'Page not found' });
    await user.click(screen.getByRole('link', { name: 'Return to Office' }));
    await screen.findByRole('heading', { name: 'No world connected' });
    expect(router.state.location.pathname).toBe('/');
  });

  it('does not retain UI state across separate mounted apps', async () => {
    const user = userEvent.setup();
    const first = renderOffice();
    await screen.findByRole('heading', { name: 'No world connected' });
    await user.click(screen.getByRole('button', { name: 'Preview details' }));
    expect(screen.getByText(/This shell does not create a world/)).toBeDefined();
    first.unmount();
    renderOffice();
    await screen.findByRole('heading', { name: 'No world connected' });
    expect(
      screen.getByRole('button', { name: 'Preview details' }).getAttribute('aria-expanded')
    ).toBe('false');
    expect(screen.queryByText(/This shell does not create a world/)).toBeNull();
  });
});
