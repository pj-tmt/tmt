import { useState } from 'react';
import { MessageSquare } from 'lucide-react';
import {
  BrowserAvatar,
  BrowserChoice,
  BrowserChoiceGroup,
  BrowserCountChip,
  BrowserMenu,
  BrowserPager,
  BrowserStatusTile,
  BrowserStatusTiles,
  type BrowserMenuItem,
} from '../src/react.js';
import { Cell, Cells, Pair, Section, noop } from './kit.js';

const needsYou: BrowserMenuItem[] = [
  { key: 'q1', label: 'How should navigation behave?' },
  { key: 'q2', label: 'When should a discussion open?' },
  { key: 'q3', label: 'Where does waiting appear?' },
];
const diamond = <span aria-hidden="true">◆</span>;

/** The count chip as a button and as a menu trigger, in each tone and state. */
export function CountChipSection() {
  return (
    <Section id="count-chip">
      <Pair
        title="BrowserCountChip"
        note="A chip with a counted number; neutral and attention tones."
      >
        {() => (
          <Cells>
            <Cell label="neutral">
              <BrowserCountChip
                label="Comments"
                count={2}
                icon={<MessageSquare className="tmt-ui-icon" aria-hidden="true" />}
                onActivate={noop}
              />
            </Cell>
            <Cell label="hover" pin="hover">
              <BrowserCountChip label="Comments" count={2} onActivate={noop} />
            </Cell>
            <Cell label="focus" pin="focus">
              <BrowserCountChip label="Comments" count={2} onActivate={noop} />
            </Cell>
            <Cell label="attention">
              <BrowserCountChip
                label="Needs you"
                count={5}
                tone="attention"
                icon={diamond}
                onActivate={noop}
              />
            </Cell>
            <Cell label="attention hover" pin="hover">
              <BrowserCountChip
                label="Needs you"
                count={5}
                tone="attention"
                icon={diamond}
                onActivate={noop}
              />
            </Cell>
            <Cell label="no count">
              <BrowserCountChip label="Comments" onActivate={noop} />
            </Cell>
            <Cell label="over 99">
              <BrowserCountChip label="Comments" count={120} onActivate={noop} />
            </Cell>
            <Cell label="disabled">
              <BrowserCountChip label="Comments" count={2} disabled onActivate={noop} />
            </Cell>
          </Cells>
        )}
      </Pair>
      <Pair
        title="BrowserMenu · count chip"
        note="Live: the chip opens the real menu; the count is in the accessible name."
      >
        {() => (
          <Cells>
            <Cell label="attention">
              <BrowserMenu
                variant="count"
                label="Needs you"
                triggerText="Needs you"
                count={3}
                tone="attention"
                icon={diamond}
                items={needsYou}
                onSelect={noop}
              />
            </Cell>
            <Cell label="attention hover" pin="hover">
              <BrowserMenu
                variant="count"
                label="Needs you"
                triggerText="Needs you"
                count={3}
                tone="attention"
                icon={diamond}
                items={needsYou}
                onSelect={noop}
              />
            </Cell>
            <Cell label="attention focus" pin="focus">
              <BrowserMenu
                variant="count"
                label="Needs you"
                triggerText="Needs you"
                count={3}
                tone="attention"
                icon={diamond}
                items={needsYou}
                onSelect={noop}
              />
            </Cell>
            <Cell label="neutral">
              <BrowserMenu
                variant="count"
                label="Threads"
                triggerText="Threads"
                count={4}
                items={needsYou}
                onSelect={noop}
              />
            </Cell>
            <Cell label="disabled">
              <BrowserMenu
                variant="count"
                label="Needs you"
                triggerText="Needs you"
                count={0}
                items={[]}
                onSelect={noop}
              />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

export function AvatarSection() {
  return (
    <Section id="avatar">
      <Pair
        title="BrowserAvatar"
        note="Initials from a name or handle; round in both themes. A person is the neutral fill, an agent the mint selection fill."
      >
        {() => (
          <Cells>
            <Cell label="m · person">
              <BrowserAvatar name="Ben" />
            </Cell>
            <Cell label="m · agent">
              <BrowserAvatar name="tmt-ux-lead" kind="agent" />
            </Cell>
            <Cell label="s · person">
              <BrowserAvatar name="Ben" size="s" />
            </Cell>
            <Cell label="s · agent">
              <BrowserAvatar name="core-3" kind="agent" size="s" />
            </Cell>
            <Cell label="decorative, name beside">
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
                <BrowserAvatar name="tmt-ux-lead" kind="agent" size="s" decorative />
                tmt-ux-lead
              </span>
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

const flavours = [
  ['Contextual links', 'Module links appear when they are useful.'],
  ['Compact rail', 'A small, persistent rail makes movement predictable.'],
  ['Let the page decide', 'The page brings its own navigation.'],
] as const;

export function ChoiceSection() {
  const [picked, setPicked] = useState('Compact rail');
  return (
    <Section id="choice">
      <Pair title="BrowserChoice · radio" note="Live: arrow keys move inside the group.">
        {(theme) => (
          <Cells>
            <Cell label="group with sublabels">
              <BrowserChoiceGroup legend="Choose one answer">
                {flavours.map(([label, description]) => (
                  <BrowserChoice
                    key={label}
                    type="radio"
                    name={`flavour-${theme}`}
                    label={label}
                    description={description}
                    checked={picked === label}
                    onChange={() => setPicked(label)}
                  />
                ))}
              </BrowserChoiceGroup>
            </Cell>
            <Cell label="inline, no sublabel">
              <BrowserChoiceGroup legend="Navigation" layout="inline">
                {flavours.map(([label]) => (
                  <BrowserChoice
                    key={label}
                    type="radio"
                    name={`inline-${theme}`}
                    label={label}
                    defaultChecked={label === 'Contextual links'}
                  />
                ))}
              </BrowserChoiceGroup>
            </Cell>
            <Cell label="hover" pin="hover">
              <BrowserChoice type="radio" name={`hover-${theme}`} label="Compact rail" />
            </Cell>
            <Cell label="focus" pin="focus">
              <BrowserChoice
                type="radio"
                name={`focus-${theme}`}
                label="Compact rail"
                defaultChecked
              />
            </Cell>
            <Cell label="disabled">
              <BrowserChoice
                type="radio"
                name={`off-${theme}`}
                label="Compact rail"
                description="Not available on this page."
                disabled
              />
            </Cell>
          </Cells>
        )}
      </Pair>
      <Pair title="BrowserChoice · check">
        {() => (
          <Cells>
            <Cell label="group with sublabels">
              <BrowserChoiceGroup legend="Notify me about">
                <BrowserChoice
                  type="checkbox"
                  label="Needs you"
                  description="A question or proposal waits for you."
                  defaultChecked
                />
                <BrowserChoice
                  type="checkbox"
                  label="Replies"
                  description="Someone answers a thread you started."
                />
                <BrowserChoice
                  type="checkbox"
                  label="Resolved"
                  description="A thread you follow is resolved."
                  disabled
                />
              </BrowserChoiceGroup>
            </Cell>
            <Cell label="focus" pin="focus">
              <BrowserChoice type="checkbox" label="Replies" defaultChecked />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

export function PagerSection() {
  const [position, setPosition] = useState(2);
  return (
    <Section id="pager">
      <Pair title="BrowserPager" note="Live: the arrows stop at the ends.">
        {() => (
          <Cells>
            <Cell label="2 of 3">
              <BrowserPager
                label="Thread"
                position={position}
                count={3}
                previousLabel="Previous thread"
                nextLabel="Next thread"
                onPrevious={() => setPosition((value) => Math.max(1, value - 1))}
                onNext={() => setPosition((value) => Math.min(3, value + 1))}
              />
            </Cell>
            <Cell label="first">
              <BrowserPager
                label="Thread"
                position={1}
                count={3}
                previousLabel="Previous thread"
                nextLabel="Next thread"
                onPrevious={noop}
                onNext={noop}
              />
            </Cell>
            <Cell label="last">
              <BrowserPager
                label="Thread"
                position={3}
                count={3}
                previousLabel="Previous thread"
                nextLabel="Next thread"
                onPrevious={noop}
                onNext={noop}
              />
            </Cell>
            <Cell label="only one">
              <BrowserPager
                label="Thread"
                position={1}
                count={1}
                previousLabel="Previous thread"
                nextLabel="Next thread"
                onPrevious={noop}
                onNext={noop}
              />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

export function StatusTileSection() {
  return (
    <Section id="status-tile">
      <Pair
        title="BrowserStatusTile"
        note="A read-only fact: the dot's colour is a hint, the value is the meaning."
      >
        {() => (
          <Cells>
            <Cell label="agent status">
              <div style={{ width: 'min(100%, 28rem)' }}>
                <BrowserStatusTiles>
                  <BrowserStatusTile label="Page" value="Available" tone="working" />
                  <BrowserStatusTile label="Session" value="Read" tone="info" />
                  <BrowserStatusTile label="Directory" value="Read" tone="info" />
                </BrowserStatusTiles>
              </div>
            </Cell>
            <Cell label="counts">
              <div style={{ width: 'min(100%, 28rem)' }}>
                <BrowserStatusTiles>
                  <BrowserStatusTile numeric label="Needs you" value="2" tone="waiting" />
                  <BrowserStatusTile numeric label="Blocked" value="1" tone="blocked" />
                  <BrowserStatusTile numeric label="Offline" value="0" />
                  <BrowserStatusTile numeric label="Working" value="3" tone="working" />
                </BrowserStatusTiles>
              </div>
            </Cell>
            <Cell label="state marks (one 12px box each)">
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '1rem', alignItems: 'center' }}>
                {['dot', 'ring', 'dashed', 'target', 'diamond', 'cross'].map((shape) => (
                  <span
                    key={shape}
                    style={{ display: 'inline-flex', gap: '0.4rem', alignItems: 'center' }}
                  >
                    <span className="tmt-ui-state-mark" data-shape={shape} aria-hidden="true" />
                    {shape}
                  </span>
                ))}
              </div>
            </Cell>
            <Cell label="other tones">
              <div style={{ width: 'min(100%, 28rem)' }}>
                <BrowserStatusTiles>
                  <BrowserStatusTile label="Message" value="Waiting" tone="waiting" />
                  <BrowserStatusTile label="Agent" value="Blocked" tone="blocked" />
                  <BrowserStatusTile label="Review" value="Needs review" tone="review" />
                  <BrowserStatusTile label="Machine" value="Offline" />
                </BrowserStatusTiles>
              </div>
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}
