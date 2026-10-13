import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeUrl, parsePairLink } from '../src/lib/pair.ts';
import { sections, segments } from '../src/lib/format.ts';
import type { SessionSummary } from '../src/lib/types.ts';

test('reads the pairing link runo --phone prints', () => {
  assert.deepEqual(parsePairLink('  runo://pair?url=http%3A%2F%2F100.64.0.2%3A7631&token=abc123 '), {
    url: 'http://100.64.0.2:7631',
    token: 'abc123',
  });
  assert.equal(parsePairLink('runo://pair?url=http%3A%2F%2Fx'), null);
  assert.equal(parsePairLink('https://example.com'), null);
  assert.equal(parsePairLink('runo://pair?url=%E0%A4%A&token=t'), null);
});

test('adds http:// and drops trailing slashes', () => {
  assert.equal(normalizeUrl('192.168.1.4:7631/'), 'http://192.168.1.4:7631');
  assert.equal(normalizeUrl('https://mac.tail1234.ts.net'), 'https://mac.tail1234.ts.net');
  assert.equal(normalizeUrl('  '), '');
});

const session = (id: number, more: Partial<SessionSummary> = {}): SessionSummary => ({
  id,
  project: 1,
  title: `S${id}`,
  agent: 'Claude Code',
  branch: '',
  status: 'done',
  unread: false,
  pinned: false,
  archived: false,
  elsewhere: false,
  updated: '',
  preview: '',
  needs: null,
  ...more,
});

test('puts what needs you first, then running work, then projects', () => {
  const list = sections(
    [
      { id: 1, name: 'runo', color: '' },
      { id: 2, name: 'cove', color: '' },
    ],
    [
      session(1),
      session(2, { status: 'working' }),
      session(3, { needs: { kind: 'plan', text: 'Plan' } }),
      session(4, { project: 2, pinned: true }),
      session(5, { project: 2 }),
      session(6, { archived: true }),
    ],
  );
  assert.deepEqual(
    list.map((s) => [s.title, s.data.map((d) => d.id)]),
    [
      ['Needs you', [3]],
      ['Working', [2]],
      ['runo', [1]],
      ['cove', [4, 5]],
    ],
  );
});

test('splits code blocks out of replies', () => {
  assert.deepEqual(segments('Run this:\n```sh\ncargo test\n```\nThen push.'), [
    { code: false, text: 'Run this:' },
    { code: true, lang: 'sh', text: 'cargo test' },
    { code: false, text: 'Then push.' },
  ]);
  assert.deepEqual(segments('plain'), [{ code: false, text: 'plain' }]);
});
