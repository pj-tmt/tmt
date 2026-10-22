import { useState } from 'react';
import { Badge, Button, Checkbox, Input, Switch, Textarea, UiTheme } from '../src/ui/index.js';
import { Cell, Cells, Pair, Section } from './kit.js';

function Switches() {
  const [on, setOn] = useState(true);
  const [checked, setChecked] = useState<boolean | 'indeterminate'>(true);
  return (
    <>
      <Cell label="switch">
        <Switch checked={on} onCheckedChange={setOn} aria-label="Notify me" />
      </Cell>
      <Cell label="checkbox">
        <Checkbox checked={checked} onCheckedChange={setChecked} aria-label="Select" />
      </Cell>
    </>
  );
}

/** The scoped shadcn layer (`@tmt/browser-ui/ui`), themed by its own `UiTheme` island. */
export function UiLayerSection() {
  return (
    <Section id="ui-layer">
      <Pair
        title="Primitives (@tmt/browser-ui/ui)"
        note="A separate scoped entry with ui.css; Remote's header uses it. Not part of the react entry."
      >
        {(theme) => (
          <UiTheme theme={theme}>
            <Cells>
              <Cell label="button">
                <Button type="button">Save draft</Button>
              </Cell>
              <Cell label="hover" pin="hover">
                <Button type="button" variant="outline">
                  Outline
                </Button>
              </Cell>
              <Cell label="focus" pin="focus">
                <Button type="button" variant="secondary">
                  Secondary
                </Button>
              </Cell>
              <Cell label="disabled">
                <Button type="button" disabled>
                  Disabled
                </Button>
              </Cell>
              <Cell label="busy">
                <Button type="button" busy busyLabel="Saving" />
              </Cell>
              <Cell label="destructive">
                <Button type="button" variant="destructive">
                  Delete
                </Button>
              </Cell>
              <Cell label="input">
                <Input aria-label="Title" defaultValue="Release plan" />
              </Cell>
              <Cell label="input focus" pin="focus">
                <Input aria-label="Title focused" defaultValue="Release plan" />
              </Cell>
              <Cell label="textarea">
                <Textarea aria-label="Comment" defaultValue="A comment" />
              </Cell>
              <Switches />
              <Cell label="badge">
                <Badge variant="outline">◆ needs you</Badge>
              </Cell>
            </Cells>
          </UiTheme>
        )}
      </Pair>
    </Section>
  );
}
