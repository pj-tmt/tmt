import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import {
  Button,
  buttonVariants,
  Input,
  Textarea,
  Checkbox,
  Switch,
  Badge,
  badgeVariants,
  Alert,
  AlertTitle,
  AlertDescription,
  Separator,
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
  tabsListVariants,
} from '../src/ui/primitives.js';

it('keeps busy action copy visible and announced while native activation is disabled', () => {
  const html = renderToStaticMarkup(
    <Button type="button" busy busyLabel="Saving changes">
      Save
    </Button>,
  );
  expect(html).toContain('Saving changes');
  expect(html).toContain('aria-busy="true"');
  expect(html).toContain('aria-disabled="true"');
  expect(html).toContain('disabled=""');
  expect(html).not.toContain('>Save<');
  expect(() =>
    renderToStaticMarkup(
      <Button asChild busy busyLabel="Saving">
        <a href="#">Save</a>
      </Button>,
    ),
  ).toThrow('native button');
});
it('forwards native field identity, value and admission attributes without wrappers or mirror state', () => {
  const input = renderToStaticMarkup(
    <Input
      id="stable"
      name="title"
      value="Kept"
      readOnly
      aria-invalid="true"
      aria-describedby="error"
    />,
  );
  for (const attribute of [
    'id="stable"',
    'name="title"',
    'value="Kept"',
    'readOnly=""',
    'aria-invalid="true"',
    'aria-describedby="error"',
  ])
    expect(input).toContain(attribute);
  expect(input.startsWith('<input')).toBe(true);
  const text = renderToStaticMarkup(<Textarea id="text" defaultValue="Retained text" disabled />);
  expect(text).toContain('id="text"');
  expect(text).toContain('disabled=""');
  expect(text).toContain('Retained text');
});
it('keeps actual Radix checkbox/switch semantics including mixed and read-only admission', () => {
  const mixed = renderToStaticMarkup(<Checkbox checked="indeterminate" aria-label="Mixed" />);
  expect(mixed).toContain('role="checkbox"');
  expect(mixed).toContain('aria-checked="mixed"');
  const readOnly = renderToStaticMarkup(
    <Checkbox readOnly defaultChecked aria-label="Kept check" />,
  );
  expect(readOnly).toContain('aria-readonly="true"');
  expect(readOnly).toContain('aria-checked="true"');
  const toggle = renderToStaticMarkup(<Switch readOnly checked aria-label="Kept switch" />);
  expect(toggle).toContain('role="switch"');
  expect(toggle).toContain('aria-readonly="true"');
  expect(toggle).toContain('aria-checked="true"');
});
it('renders stock information and tab semantics with their original Radix relationships', () => {
  const alert = renderToStaticMarkup(
    <Alert>
      <AlertTitle>Failed</AlertTitle>
      <AlertDescription>Retained reason</AlertDescription>
    </Alert>,
  );
  expect(alert).toContain('role="alert"');
  expect(alert).toContain('Retained reason');
  expect(renderToStaticMarkup(<Badge>Waiting</Badge>)).toContain('Waiting');
  expect(renderToStaticMarkup(<Separator decorative={false} />)).toContain('role="separator"');
  const tabs = renderToStaticMarkup(
    <Tabs value="one">
      <TabsList>
        <TabsTrigger value="one">One</TabsTrigger>
        <TabsTrigger value="two" disabled>
          Two
        </TabsTrigger>
      </TabsList>
      <TabsContent value="one">Kept panel</TabsContent>
    </Tabs>,
  );
  expect(tabs).toContain('role="tablist"');
  expect(tabs).toContain('role="tabpanel"');
  expect(tabs).toContain('aria-selected="true"');
  expect(tabs).toContain('Kept panel');
});
it('uses genuine variant functions and only complete prefixed utilities', () => {
  for (const classes of [
    buttonVariants({ variant: 'outline', size: 'sm' }),
    badgeVariants({ variant: 'destructive' }),
    tabsListVariants({ variant: 'line' }),
  ]) {
    expect(classes.split(/\s+/).every((name: string) => name.startsWith('tmt:'))).toBe(true);
    expect(classes).not.toMatch(/(?:^|:)(?:rounded|shadow|ring)(?:-|$)/);
    expect(classes).not.toMatch(/text-white|bg-black/);
  }
});
