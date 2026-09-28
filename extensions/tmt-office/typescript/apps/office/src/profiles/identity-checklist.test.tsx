import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { IdentityChecklist } from './identity-checklist.js';

it('distinguishes unknown, offline and missing choices without treating presence as permission', async () => {
  const onChange = vi.fn();
  const missing = { id: 'missing', name: 'Missing' };
  render(
    <IdentityChecklist
      legend="Recipients"
      choices={[
        { id: 'online', name: 'Online', presence: 'active' },
        { id: 'offline', name: 'Offline', presence: 'offline' },
        { id: 'unknown', name: 'Unknown', presence: 'unknown' },
      ]}
      selected={[missing]}
      limit={4}
      onChange={onChange}
    />
  );
  const user = userEvent.setup();
  for (const [label, id, name] of [
    ['Online', 'online', 'Online'],
    ['Offline · offline', 'offline', 'Offline'],
    ['Unknown · presence unknown', 'unknown', 'Unknown'],
  ]) {
    await user.click(screen.getByRole('checkbox', { name: label }));
    expect(onChange).toHaveBeenLastCalledWith([missing, { id, name }]);
  }
  await user.click(
    screen.getByRole('checkbox', { name: 'Missing · selected, not in current list' })
  );
  expect(onChange).toHaveBeenLastCalledWith([]);
});
