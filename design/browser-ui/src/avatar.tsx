import { avatarInitials, browserUiClasses as c } from './static.js';

export interface BrowserAvatarProps {
  /** A person's or agent's name or handle; the avatar draws its initials. */
  name: string;
  /** A person keeps the neutral fill; an agent is drawn in the mint selection fill. */
  kind?: 'person' | 'agent';
  size?: 'm' | 's';
  /** The name is written next to the avatar, so the avatar adds nothing to a screen reader. */
  decorative?: boolean;
}

/** A round initials mark for a person or an agent. */
export function BrowserAvatar({
  name,
  kind = 'person',
  size = 'm',
  decorative = false,
}: BrowserAvatarProps) {
  return (
    <span
      className={c.avatar}
      data-kind={kind}
      data-size={size}
      {...(decorative ? { 'aria-hidden': 'true' } : { role: 'img', 'aria-label': name })}
    >
      {avatarInitials(name)}
    </span>
  );
}
