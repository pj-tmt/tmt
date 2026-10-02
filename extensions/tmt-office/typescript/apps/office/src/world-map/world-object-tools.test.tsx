import { fireEvent, render as mount, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vite-plus/test';
import { officeWorldFixture } from '../../../../../../../typescript/test/support/office-world.js';
import { WorldObjectTools } from './world-object-tools.js';
import type { WorldObject } from './world-contract.js';

// Coordinate scenarios explicitly enter the precision controls, as a user does.
function render(ui: Parameters<typeof mount>[0]) {
  const view = mount(ui);
  fireEvent.click(screen.getByText('Precise placement', { selector: 'summary' }));
  return view;
}

it('offers floor placement and actions without wall settings on platforms', () => {
  render(
    <WorldObjectTools
      identities={[]}
      object={officeWorldFixture().layout.objects[0]!}
      change={vi.fn()}
      remove={vi.fn()}
      wallEditing={false}
    />
  );
  expect(screen.queryByLabelText('Placement surface')).toBeNull();
  expect(screen.queryByLabelText('Object kind')).toBeNull();
  expect(screen.queryByLabelText('Wall direction')).toBeNull();
  expect(screen.getByRole('form', { name: 'Object coordinates' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Rotate 90° clockwise' })).toBeTruthy();
});

it('keeps the precision rotation control discoverable when art is unavailable', async () => {
  const change = vi.fn();
  render(
    <WorldObjectTools
      identities={[]}
      object={officeWorldFixture().layout.objects[0]!}
      change={change}
      remove={vi.fn()}
      canRotate={false}
    />
  );
  const button = screen.getByRole('button', { name: 'Rotate 90° clockwise' });
  expect(button).toHaveProperty('disabled', true);
  expect(button.getAttribute('title')).toBe('Directional artwork is unavailable');
  await userEvent.click(button);
  expect(change).not.toHaveBeenCalled();
});

it('does not dispatch a rotation rejected by the shared placement validator', async () => {
  const rotate = vi.fn();
  const change = vi.fn();
  render(
    <WorldObjectTools
      identities={[]}
      object={officeWorldFixture().layout.objects[0]!}
      change={change}
      remove={vi.fn()}
      rotate={rotate}
      rotationProblem="Keep the entire object on the platform"
    />
  );
  const button = screen.getByRole('button', { name: 'Rotate 90° clockwise' });
  expect(button).toHaveProperty('disabled', true);
  expect(screen.getByRole('status').textContent).toBe('Keep the entire object on the platform');
  await userEvent.click(button);
  expect(rotate).not.toHaveBeenCalled();
  expect(change).not.toHaveBeenCalled();
});

it('starts with precision settings collapsed while common actions remain available', async () => {
  const user = userEvent.setup();
  const object = officeWorldFixture().layout.objects[0]!;
  const change = vi.fn();
  mount(<WorldObjectTools identities={[]} object={object} change={change} remove={vi.fn()} />);
  const summary = screen.getByText('Precise placement', { selector: 'summary' });
  expect(summary.parentElement?.hasAttribute('open')).toBe(false);
  expect(
    screen.getByText('Object action', { selector: 'summary' }).parentElement?.hasAttribute('open')
  ).toBe(false);
  expect(
    screen.getByRole('form', { name: 'Web link' }).closest('details')?.hasAttribute('open')
  ).toBe(false);
  await user.click(summary);
  await user.click(screen.getByRole('button', { name: 'Rotate 90° clockwise' }));
  expect(change).toHaveBeenCalledExactlyOnceWith({
    ...object,
    placement: {
      ...object.placement,
      x: object.placement.x + 1,
      y: object.placement.y - 1,
      rotation: (object.placement.rotation + 1) % 4,
    },
  });
  expect(summary.parentElement?.hasAttribute('open')).toBe(true);
  expect(screen.getByRole('form', { name: 'Object coordinates' })).toBeTruthy();
  await user.click(screen.getByText('Object action', { selector: 'summary' }));
  expect(screen.getByRole('form', { name: 'Web link' })).toBeTruthy();
});

it('keeps linked-object settings opt-in and resets disclosure when selecting another object', async () => {
  const user = userEvent.setup();
  const object: WorldObject = {
    ...officeWorldFixture().layout.objects[0]!,
    extension: {
      definition: 'tmt-link',
      binding: { kind: 'external-link', url: 'https://example.com/docs' },
    },
  };
  const change = vi.fn();
  const remove = vi.fn();
  const view = mount(
    <WorldObjectTools identities={[]} object={object} change={change} remove={remove} />
  );
  const disclosure = () =>
    screen.getByText('Object action', { selector: 'summary' }).parentElement!;
  expect(disclosure().hasAttribute('open')).toBe(false);
  await user.click(screen.getByText('Object action', { selector: 'summary' }));
  expect(disclosure().hasAttribute('open')).toBe(true);
  expect(screen.getByLabelText('Web destination')).toHaveProperty(
    'value',
    'https://example.com/docs'
  );
  view.rerender(
    <WorldObjectTools identities={[]} object={{ ...object }} change={change} remove={remove} />
  );
  expect(disclosure().hasAttribute('open')).toBe(true);
  view.rerender(
    <WorldObjectTools
      identities={[]}
      object={{ ...object, id: 'another-object' }}
      change={change}
      remove={remove}
    />
  );
  expect(disclosure().hasAttribute('open')).toBe(false);
  expect(change).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
});

it.each([0, 1])('bounds elevation by the actual rotated footprint (rotation %s)', (rotation) => {
  const source = officeWorldFixture().layout.objects[0]!;
  const object: WorldObject = {
    ...source,
    placement: { ...source.placement, rotation },
    surface: { type: 'wall', axis: 'horizontal', face: 'positive', elevation: 0 },
  };
  const max = rotation === 0 ? 14 : 12; // The literal desk footprint is 4 by 2.
  const change = vi.fn();
  render(<WorldObjectTools identities={[]} object={object} change={change} remove={vi.fn()} />);
  const elevation = screen.getByLabelText('Elevation');
  expect(elevation.getAttribute('max')).toBe(String(max));
  fireEvent.change(elevation, { target: { value: String(max + 1) } });
  fireEvent.submit(screen.getByRole('form', { name: 'Object coordinates' }));
  expect(change).not.toHaveBeenCalled();
  fireEvent.change(elevation, { target: { value: String(max) } });
  fireEvent.submit(screen.getByRole('form', { name: 'Object coordinates' }));
  expect(change).toHaveBeenCalledExactlyOnceWith({
    ...object,
    surface: { ...object.surface, elevation: max },
  });
});

it('keeps incomplete input outside world history, applies one signed edit, and resets on authoritative undo', async () => {
  const user = userEvent.setup();
  const object = officeWorldFixture().layout.objects[0]!;
  const change = vi.fn();
  const remove = vi.fn();
  const view = render(
    <WorldObjectTools identities={[]} object={object} change={change} remove={remove} />
  );
  const x = screen.getByLabelText('X');
  await user.clear(x);
  await user.type(x, '-');
  expect(change).not.toHaveBeenCalled();
  await user.type(x, '12');
  await user.click(screen.getByRole('button', { name: 'Apply coordinates' }));
  expect(change).toHaveBeenCalledExactlyOnceWith({
    ...object,
    placement: { ...object.placement, x: -12 },
  });
  const next = change.mock.calls[0]![0];
  view.rerender(<WorldObjectTools identities={[]} object={next} change={change} remove={remove} />);
  expect(screen.getByLabelText('X')).toHaveProperty('value', '-12');
  view.rerender(
    <WorldObjectTools identities={[]} object={object} change={change} remove={remove} />
  );
  expect(screen.getByLabelText('X')).toHaveProperty('value', '4');
  expect(remove).not.toHaveBeenCalled();
});

it.each(['', '-4097', '4097', '0.5'])('does not apply invalid coordinate %j', (value) => {
  const change = vi.fn();
  render(
    <WorldObjectTools
      identities={[]}
      object={officeWorldFixture().layout.objects[0]!}
      change={change}
      remove={vi.fn()}
    />
  );
  fireEvent.change(screen.getByLabelText('X'), { target: { value } });
  fireEvent.submit(screen.getByRole('form', { name: 'Object coordinates' }));
  expect(change).not.toHaveBeenCalled();
});

it.each([-4096, 4096])(
  'accepts the coordinate boundary %s without changing object identity or resources',
  (x) => {
    const object = officeWorldFixture().layout.objects[0]!;
    const change = vi.fn();
    render(<WorldObjectTools identities={[]} object={object} change={change} remove={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('X'), { target: { value: String(x) } });
    fireEvent.submit(screen.getByRole('form', { name: 'Object coordinates' }));
    expect(change).toHaveBeenCalledExactlyOnceWith({
      ...object,
      placement: { ...object.placement, x },
    });
  }
);
