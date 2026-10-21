import { BrowserList, BrowserListRow, BrowserSection, BrowserTable } from '../src/react.js';
import { browserUiClasses } from '../src/static.js';
import { Pair, Section } from './kit.js';

export function PageSectionsSection() {
  return (
    <Section id="section">
      <Pair title="BrowserSection · label">
        {() => (
          <BrowserSection heading="Needs you">
            <BrowserList label="Needs you">
              <BrowserListRow
                title={<a href="#section">Release plan for the remote channel</a>}
                metadata="ux-4 · 2 minutes ago"
                state={<span>◆ needs you</span>}
              />
            </BrowserList>
          </BrowserSection>
        )}
      </Pair>
      <Pair title="BrowserSection · title, divided">
        {() => (
          <>
            <BrowserSection heading="Browser opening" variant="title" divided>
              <p>Opens the printed link in this browser.</p>
            </BrowserSection>
            <BrowserSection heading="Paired devices" variant="title" divided>
              <p>No other device is paired.</p>
            </BrowserSection>
          </>
        )}
      </Pair>
    </Section>
  );
}

export function TablesSection() {
  return (
    <Section id="table">
      <Pair title="BrowserTable · numeric">
        {() => (
          <BrowserTable
            label="Usage by agent"
            numeric
            columns={['Agent', 'Tokens', 'Share', 'Turns']}
            rows={[
              ['ux-2', '1.2M', '61%', '48'],
              ['ux-4', '760K', '39%', '31'],
            ]}
          />
        )}
      </Pair>
      <Pair title="BrowserTable · text">
        {() => (
          <BrowserTable
            label="Published limits"
            columns={['Limit', 'Published allowance']}
            rows={[
              ['Document reads', '50,000 per day'],
              ['Stored data', '1 GiB'],
            ]}
          />
        )}
      </Pair>
    </Section>
  );
}

export function VisuallyHiddenSection() {
  return (
    <Section id="visually-hidden">
      <Pair title="tmt-ui-visually-hidden">
        {() => (
          <p>
            Saved<span className={browserUiClasses.visuallyHidden}> — the draft is stored</span>
          </p>
        )}
      </Pair>
    </Section>
  );
}
