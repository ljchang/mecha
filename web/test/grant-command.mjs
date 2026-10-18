// The web chat's `/grant` and `/revoke`, parsed as the page parses them.
//
// `npm test` in web/. What reaches the model and what reaches the grants
// endpoint is decided here, so a mistake is either a grant command sent to
// the model as chat, or a message eaten as a command — both silent.
import { parseGrantCommand, revocable } from '../src/lib/grants.js';

let failed = 0;
function check(name, got, want) {
  const ok = JSON.stringify(got) === JSON.stringify(want);
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : `: got ${JSON.stringify(got)}`}`);
  if (!ok) failed += 1;
}

check('bare /grant lists', parseGrantCommand('/grant'), { list: true });
check('bare /grants lists', parseGrantCommand('  /grants  '), { list: true });
check('a path reads by default', parseGrantCommand('/grant ~/papers'), {
  action: 'grant', path: '~/papers', access: 'read',
});
check('write as the last word', parseGrantCommand('/grant ~/My Papers write'), {
  action: 'grant', path: '~/My Papers', access: 'write',
});
check('rw spelling', parseGrantCommand('/grant /srv/data rw'), {
  action: 'grant', path: '/srv/data', access: 'write',
});
check('a path whose last word is not an access stays whole', parseGrantCommand('/grant ~/notes for school'), {
  action: 'grant', path: '~/notes for school', access: 'read',
});
check('revoke takes the whole path', parseGrantCommand('/revoke ~/My Papers'), {
  action: 'revoke', path: '~/My Papers',
});
check('a bare revoke is an error, not a request', parseGrantCommand('/revoke').error !== undefined, true);
check('not a grant command: ordinary text', parseGrantCommand('grant me access'), null);
check('not a grant command: another word', parseGrantCommand('/granted, thanks'), null);
check('not a grant command: another command', parseGrantCommand('/send x'), null);
check('a config grant is not revocable here', revocable({ source: 'config' }), false);
check('an owner grant is', revocable({ source: 'owner' }), true);

if (failed) {
  console.error(`${failed} grant-command check(s) failed`);
  process.exit(1);
}
