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
import { shellIconNodes, type ShellIcon } from '../src/shell-icons.js';
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
            <Cell label="with a key hint">
              <BrowserChoiceGroup legend="Pick with a digit key">
                {flavours.map(([label], index) => (
                  <BrowserChoice
                    key={label}
                    type="radio"
                    name={`hint-${theme}`}
                    label={label}
                    hint={String(index + 1)}
                    checked={picked === label}
                    onChange={() => setPicked(label)}
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
                hint="4"
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
                {['dot', 'ring', 'dashed', 'clock', 'diamond', 'cross', 'unknown'].map((shape) => (
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

/** The flat card with a head, bare facts and a meter, and the meter at its ends. */
export function CardSection() {
  return (
    <Section id="card">
      <Pair
        title="tmt-ui-card"
        note="A flat sheet that groups one subject: a head (title, optional note) over its body. Facts inside it are bare tiles; the meter is a read-only measure coloured by a chart series."
      >
        {() => (
          <Cells>
            <Cell label="card with bare facts and a meter">
              <div style={{ width: 'min(100%, 28rem)' }}>
                <section className="tmt-ui-card" aria-labelledby="gallery-card-title">
                  <div className="tmt-ui-card-head">
                    <h3 className="tmt-ui-card-title" id="gallery-card-title">
                      Claude
                    </h3>
                    <span className="tmt-ui-card-meta">Resets in 3d 4h</span>
                  </div>
                  <dl className="tmt-ui-status-tiles" data-bare="">
                    <div className="tmt-ui-status-tile">
                      <dt className="tmt-ui-status-tile-label">Allowance left</dt>
                      <dd
                        className="tmt-ui-status-tile-value"
                        data-numeric="count"
                        data-size="hero"
                      >
                        <span
                          className="tmt-ui-status-tile-dot tmt-ui-chart-s1"
                          aria-hidden="true"
                        />
                        72%
                      </dd>
                    </div>
                  </dl>
                  <div
                    className="tmt-ui-meter"
                    role="img"
                    aria-label="72% of the weekly allowance left"
                  >
                    <div
                      className="tmt-ui-meter-fill tmt-ui-chart-s1"
                      style={{ ['--tmt-ui-meter-fill' as string]: 72 }}
                    />
                  </div>
                  <dl className="tmt-ui-status-tiles" data-bare="">
                    <div className="tmt-ui-status-tile">
                      <dt className="tmt-ui-status-tile-label">Elapsed</dt>
                      <dd className="tmt-ui-status-tile-value" data-numeric="count">
                        41%
                      </dd>
                    </div>
                    <div className="tmt-ui-status-tile">
                      <dt className="tmt-ui-status-tile-label">Pace</dt>
                      <dd className="tmt-ui-status-tile-value" data-numeric="count">
                        +4.2 pts
                      </dd>
                    </div>
                  </dl>
                </section>
              </div>
            </Cell>
            <Cell label="meter: empty, half, full">
              <div style={{ display: 'grid', gap: '0.75rem', width: 'min(100%, 20rem)' }}>
                {[0, 50, 100].map((fill) => (
                  <div key={fill} className="tmt-ui-meter" role="img" aria-label={`${fill}% left`}>
                    <div
                      className="tmt-ui-meter-fill tmt-ui-chart-s2"
                      style={{ ['--tmt-ui-meter-fill' as string]: fill }}
                    />
                  </div>
                ))}
              </div>
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

function Glyph({ name }: { name: ShellIcon }) {
  return (
    <svg className="tmt-ui-glyph" viewBox="0 0 24 24" aria-hidden="true">
      {shellIconNodes[name].map(([tag, attributes], index) =>
        tag === 'path' ? <path key={index} {...attributes} /> : null,
      )}
    </svg>
  );
}

const agentRows = [
  ['tmt-lead', 'diamond', 'waiting', 'Needs you', 'Review transport plan', 'opus'],
  ['core-1', 'dot', 'working', 'Working', 'Verify request lifecycle', 'sol'],
  ['core-3', 'unknown', '', 'Unknown', '', 'sonnet'],
] as const;

/** The pieces of a read-only overview page: headline, count strip, banner, facts and a grouped table. */
export function OverviewSection() {
  return (
    <Section id="overview">
      <Pair title="Page head and count strip" stacked>
        {() => (
          <div style={{ display: 'grid', gap: '1rem' }}>
            <header className="tmt-ui-page-head" style={{ margin: 0 }}>
              <h2 className="tmt-ui-page-title">Your team, in sync.</h2>
              <p className="tmt-ui-page-subtitle">A clear view of every squad and agent.</p>
            </header>
            <div className="tmt-ui-stats" role="group" aria-label="Agents by state">
              <div className="tmt-ui-stat">
                <Glyph name="users" />
                <div>
                  <p className="tmt-ui-stat-value">
                    <span className="tmt-ui-stat-number">6</span> agents
                  </p>
                  <p className="tmt-ui-stat-caption">Total in workspace</p>
                </div>
              </div>
              <div className="tmt-ui-stat" data-tone="working">
                <span className="tmt-ui-state-mark" data-shape="dot" aria-hidden="true" />
                <div>
                  <p className="tmt-ui-stat-value">
                    <span className="tmt-ui-stat-number">3</span> working
                  </p>
                  <p className="tmt-ui-stat-caption">Actively running</p>
                </div>
              </div>
              <div className="tmt-ui-stat" data-tone="waiting">
                <span className="tmt-ui-state-mark" data-shape="diamond" aria-hidden="true" />
                <div>
                  <p className="tmt-ui-stat-value">
                    <span className="tmt-ui-stat-number">1</span> needs you
                  </p>
                  <p className="tmt-ui-stat-caption">Requires attention</p>
                </div>
              </div>
              <div className="tmt-ui-stat" data-tone="muted">
                <span className="tmt-ui-state-mark" data-shape="ring" aria-hidden="true" />
                <div>
                  <p className="tmt-ui-stat-value">
                    <span className="tmt-ui-stat-number">1</span> offline
                  </p>
                  <p className="tmt-ui-stat-caption">Not running</p>
                </div>
              </div>
            </div>
          </div>
        )}
      </Pair>
      <Pair title="Banner and facts" stacked>
        {() => (
          <div style={{ display: 'grid', gap: '1rem' }}>
            <section className="tmt-ui-banner" data-tone="waiting">
              <div className="tmt-ui-banner-head">
                <Glyph name="triangle-alert" />
                <span>1 agent needs your attention</span>
                <button type="button" className="tmt-ui-action" data-variant="text">
                  <span className="tmt-ui-action-label">View all</span>
                  <Glyph name="chevron-right" />
                </button>
              </div>
              <button type="button" className="tmt-ui-banner-row">
                <span
                  className="tmt-ui-state-mark"
                  data-shape="diamond"
                  data-tone="waiting"
                  aria-hidden="true"
                />
                <span>tmt-lead</span>
                <span>
                  <span className="tmt-ui-state-mark" data-shape="diamond" aria-hidden="true" />
                  Needs you
                </span>
                <span>Approve transport plan</span>
                <span>Continue after review</span>
                <Glyph name="chevron-right" />
              </button>
            </section>
            <dl className="tmt-ui-facts" style={{ maxWidth: '20rem' }}>
              <dt>Name</dt>
              <dd>core-1</dd>
              <dt>Squad</dt>
              <dd>tmt-core</dd>
              <dt>State</dt>
              <dd>Working</dd>
            </dl>
          </div>
        )}
      </Pair>
      <Pair
        title="Avatar stack, glyph tile, read-only badge, recorded values and a copy command"
        stacked
      >
        {() => (
          <div style={{ display: 'grid', gap: '1rem', maxWidth: '36rem' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
              <span className="tmt-ui-avatar-stack" aria-hidden="true">
                {['TL', 'C1', 'CO', 'C3'].map((initials) => (
                  <span key={initials} className="tmt-ui-avatar" data-size="s">
                    {initials}
                  </span>
                ))}
              </span>
              <span className="tmt-ui-glyph-tile">
                <Glyph name="terminal" />
              </span>
              <span className="tmt-ui-chip">
                <Glyph name="lock" />
                <span className="tmt-ui-chip-label">Read only</span>
              </span>
            </div>
            <table className="tmt-ui-table" data-roomy="">
              <tbody>
                <tr>
                  <td>Project status check</td>
                  <td className="tmt-ui-table-mono">cron 35 * * * *</td>
                  <td className="tmt-ui-table-mono tmt-ui-table-quiet">status-check</td>
                </tr>
              </tbody>
            </table>
            <span className="tmt-ui-command">
              <code className="tmt-ui-command-text">tmt ops ui</code>
              <button type="button" className="tmt-ui-command-copy" aria-label="Copy tmt ops ui">
                <Glyph name="copy" />
                Copy
              </button>
              <span className="tmt-ui-command-feedback" role="status" />
            </span>
          </div>
        )}
      </Pair>
      <Pair title="Table in a flush card, with a group, a chosen row and state chips" stacked>
        {() => (
          <section className="tmt-ui-card" data-flush="">
            <div className="tmt-ui-card-head">
              <h3 className="tmt-ui-card-title">Agents</h3>
              <div className="tmt-ui-search-field">
                <Glyph name="search" />
                <input
                  type="search"
                  className="tmt-ui-field-control"
                  placeholder="Filter agents"
                  aria-label="Filter agents"
                />
              </div>
            </div>
            <div className="tmt-ui-table-scroll">
              <table className="tmt-ui-table" data-roomy="">
                <thead>
                  <tr>
                    <th scope="col">Agent</th>
                    <th scope="col">Status</th>
                    <th scope="col">Current work</th>
                    <th scope="col" data-compact-hide="">
                      Model
                    </th>
                  </tr>
                </thead>
                <tbody>
                  <tr className="tmt-ui-table-group">
                    <th colSpan={4} scope="colgroup">
                      <button
                        type="button"
                        className="tmt-ui-table-group-toggle"
                        aria-expanded="true"
                      >
                        <Glyph name="chevron-down" />
                        tmt-core
                        <span className="tmt-ui-table-group-count">· 3 agents</span>
                      </button>
                    </th>
                  </tr>
                  {agentRows.map(([name, shape, tone, word, work, model], index) => (
                    <tr key={name} data-selected={index === 1 ? '' : undefined}>
                      <td>
                        <button type="button" className="tmt-ui-table-select">
                          <span
                            className="tmt-ui-state-mark"
                            data-shape={shape}
                            aria-hidden="true"
                          />
                          {name}
                        </button>
                      </td>
                      <td>
                        <span className="tmt-ui-chip" data-tone={tone || undefined}>
                          <span
                            className="tmt-ui-state-mark"
                            data-shape={shape}
                            aria-hidden="true"
                          />
                          <span className="tmt-ui-chip-label">{word}</span>
                        </span>
                      </td>
                      <td>
                        {work || '—'}
                        <span className="tmt-ui-table-compact-meta">{model}</span>
                      </td>
                      <td data-compact-hide="">{model}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}
      </Pair>
    </Section>
  );
}
