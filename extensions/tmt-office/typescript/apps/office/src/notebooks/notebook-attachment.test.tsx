import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { NotebookAttachment } from './notebook-attachment.js';
import { PROFILE_CATALOG } from '../profiles/profile-contract.js';
import type { ProfileProjection } from '../profiles/profile-contract.js';
import type { WorldObject } from '../world-map/world-contract.js';

const identityId = '11111111-1111-4111-8111-111111111111';
const saved: ProfileProjection = {
  identityId,
  identityName: 'Alice',
  exists: false,
  revision: 0,
  lifetime: 'saved',
  presence: 'offline' as const,
  selfReportedStatus: null,
  updatedAtMs: null,
  catalog: PROFILE_CATALOG,
  profile: {
    displayLabel: '',
    description: '',
    appearance: {
      hairStyle: 'short',
      hairColor: 'ink',
      skinTone: 'medium',
      shirtColor: 'blue',
      shirtMark: '',
    },
  },
};
const temporary = {
  ...saved,
  identityId: '22222222-2222-4222-8222-222222222222',
  identityName: 'Contractor',
  lifetime: 'temporary' as const,
};
const object = officeWorldFixture().layout.objects[0]!;

it('requires explicit saved identity selection and changes only the resource reference', () => {
  const change = vi.fn();
  render(<NotebookAttachment object={object} identities={[saved, temporary]} change={change} />);
  expect(screen.queryByRole('option', { name: 'Contractor' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Attach notebook to object' })).toHaveProperty(
    'disabled',
    true
  );
  expect(change).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('Notebook owner'), { target: { value: identityId } });
  expect(change).not.toHaveBeenCalled();
  fireEvent.submit(screen.getByRole('form', { name: 'Notebook binding' }));
  expect(change).toHaveBeenCalledExactlyOnceWith({
    ...object,
    extension: { definition: 'tmt-notebook', binding: { kind: 'notebook', identityId } },
  });
});

it('retains an unavailable identity instead of redirecting to a replacement and detaches only explicitly', () => {
  const attached: WorldObject = {
    ...object,
    extension: { definition: 'tmt-notebook', binding: { kind: 'notebook', identityId } },
  };
  const change = vi.fn();
  render(
    <NotebookAttachment
      object={attached}
      identities={[{ ...saved, identityId: temporary.identityId }]}
      change={change}
    />
  );
  expect(screen.getByLabelText('Notebook owner')).toHaveProperty('value', identityId);
  expect(screen.getByRole('button', { name: 'Update notebook in draft' })).toHaveProperty(
    'disabled',
    true
  );
  fireEvent.submit(screen.getByRole('form', { name: 'Notebook binding' }));
  expect(change).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Remove notebook action' }));
  expect(change).toHaveBeenCalledExactlyOnceWith({ ...attached, extension: null });
});

it('does not replace another functional resource', () => {
  const change = vi.fn();
  const attached: WorldObject = {
    ...object,
    extension: { definition: 'tmt-discussion', binding: { kind: 'office-board' } },
  };
  const view = render(
    <NotebookAttachment object={attached} identities={[saved]} change={change} />
  );
  expect(view.container.textContent).toBe('');
  expect(change).not.toHaveBeenCalled();
});
