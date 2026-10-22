import { expect, it, vi } from 'vite-plus/test';
import {
  anonymousText,
  messageParts,
  SendIdentityNotice,
  USER_NAME_COMMAND,
} from '../src/send-identity-notice.js';

const anonymous = { state: 'anonymous', reason: 'USER_NAME_INVALID', message: 'x' } as const;
const renamed = { state: 'person', name: 'ben-user', renamedFrom: 'ben' } as const;
it('tells the page session once, in memory, and a dismissal does not bring it back', () => {
  const notice = new SendIdentityNotice();
  const changed = vi.fn();
  notice.subscribe(changed);
  expect(notice.current()).toBeNull();
  notice.show(anonymous);
  expect(notice.current()).toBe(anonymous);
  notice.show(renamed);
  expect(notice.current()).toBe(anonymous);
  notice.dismiss();
  expect(notice.current()).toBeNull();
  notice.show(renamed);
  expect(notice.current()).toBeNull();
  notice.dismiss();
  expect(changed).toHaveBeenCalledTimes(2);
});
it('stops notifying a listener that unsubscribed', () => {
  const notice = new SendIdentityNotice();
  const changed = vi.fn();
  notice.subscribe(changed)();
  notice.show(anonymous);
  expect(changed).not.toHaveBeenCalled();
});
it('splits Core’s message around the command it names, and leaves any other message whole', () => {
  expect(messageParts(`Set one with ${USER_NAME_COMMAND}.`)).toEqual({
    before: 'Set one with ',
    command: true,
    after: '.',
  });
  expect(messageParts('USER_NAME_TAKEN: pick another')).toEqual({
    before: 'USER_NAME_TAKEN: pick another',
    command: false,
    after: '',
  });
});
it('drops Core’s own code prefix only when it is this notice’s reason, and capitalises the rest', () => {
  const said = (reason: string, message: string) =>
    anonymousText({ state: 'anonymous', reason, message } as never);
  expect(said('USER_NAME_UNAVAILABLE', 'USER_NAME_UNAVAILABLE: no login name; set one.')).toBe(
    'No login name; set one.',
  );
  // Another code, or no prefix, is left whole (the first letter still capitalised).
  expect(said('USER_NAME_TAKEN', 'USER_NAME_INVALID: not a name')).toBe(
    'USER_NAME_INVALID: not a name',
  );
  expect(said('USER_NAME_TAKEN', 'already used by an agent')).toBe('Already used by an agent');
  expect(said('USER_NAME_TAKEN', 'USER_NAME_TAKEN:no space')).toBe('USER_NAME_TAKEN:no space');
  expect(said('USER_NAME_TAKEN', 'USER_NAME_TAKEN: ')).toBe('');
});
