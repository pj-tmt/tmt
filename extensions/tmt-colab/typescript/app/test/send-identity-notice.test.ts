import { expect, it, vi } from 'vite-plus/test';
import {
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
