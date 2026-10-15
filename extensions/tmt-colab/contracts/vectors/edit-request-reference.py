"""Independent known answers for the typed `edit-request` own record; stdlib only, no product imports.

A case marked `digestOnly` is refused only because the source does not hash to `sourceSha256`:
the browser fold checks shape and size (the hash needs an async digest) and `verifiedEditRequest`
adds the hash, while the native decoder checks both at once.

A string written as {"fill": text, "count": n} stands for text repeated n times (both validators
expand it the same way), so the size bounds are checked exactly without a large file.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path

DEST = Path(__file__).with_name('edit-request-v1.json')
SOURCE_BYTES = 128 * 1024
HUNK_BYTES = 64 * 1024
HUNKS = 64

def ident(n):
    return f'00000000-0000-4000-8000-{n:012x}'
def b64(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b'=').decode()
def sha(text):
    return b64(hashlib.sha256(text.encode()).digest())
def fill(text, count):
    return {'fill': text, 'count': count}
def expand(value):
    if isinstance(value, dict) and set(value) == {'fill', 'count'}:
        return value['fill'] * value['count']
    return value

BASE = sha('<p>base</p>\n')
def record(op=1, source='<p>edited</p>\n', hunks=None, digest=None, **change):
    text = expand(source)
    value = dict(version=1, kind='edit-request', operationId=ident(op), baseSha256=BASE,
                 sourceSha256=digest or sha(text), source=source)
    if hunks is not None:
        value['hunks'] = hunks
    value.update(change)
    return value
def case(name, valid, value, root='intents', key=None):
    return dict(name=name, valid=valid, root=root, key=value.get('operationId') if key is None else key, record=value)

def generate():
    hunk = dict(oldStart=1, old='<p>base</p>\n', new='<p>edited</p>\n')
    # The largest request: a source of exactly SOURCE_BYTES (two-byte characters) and 64 hunks whose
    # old and new text together are exactly HUNK_BYTES.
    wide = fill('é', SOURCE_BYTES // 2)
    wide_hunks = [dict(oldStart=i + 1, old=fill('o', 512), new=fill('n', 512)) for i in range(HUNKS)]
    cases = [
        case('minimal', True, record()),
        case('with-hunks', True, record(hunks=[hunk, dict(oldStart=4, old='a\n', new='b\nc\n')])),
        case('maximal', True, record(source=wide, hunks=wide_hunks, digest=sha(expand(wide)))),
        case('source-one-byte-over', False, record(source=fill('a', SOURCE_BYTES + 1))),
        case('hunk-text-one-byte-over', False, record(hunks=[dict(oldStart=1, old=fill('o', HUNK_BYTES // 2), new=fill('n', HUNK_BYTES // 2 + 1))])),
        case('too-many-hunks', False, record(hunks=[dict(oldStart=i + 1, old='a', new='b') for i in range(HUNKS + 1)])),
        dict(case('source-hash-differs', False, record(digest=sha('another source'))), digestOnly=True),
        case('base-hash-padded', False, record(baseSha256=BASE + '=')),
        case('base-hash-short', False, record(baseSha256=b64(b'\x00' * 31))),
        case('wrong-root', False, record(), root='messages'),
        case('key-is-not-the-operation', False, record(), key=ident(2)),
        case('operation-is-not-an-id', False, record(operationId='not-an-id'), key='not-an-id'),
        case('version-two', False, record(version=2)),
        case('extra-field', False, record(extra=True)),
        case('empty-hunks', False, record(hunks=[])),
        case('null-hunks', False, {**record(), 'hunks': None}),
        case('empty-old', False, record(hunks=[dict(oldStart=1, old='', new='x')])),
        case('hunk-starts-at-zero', False, record(hunks=[dict(oldStart=0, old='a', new='b')])),
        case('hunk-extra-field', False, record(hunks=[dict(oldStart=1, old='a', new='b', note='x')])),
        case('source-not-a-string', False, record(source=3, digest=sha(''))),
    ]
    assert len({c['name'] for c in cases}) == len(cases)
    return dict(version=1, sourceBytes=SOURCE_BYTES, hunkBytes=HUNK_BYTES, hunks=HUNKS, cases=cases)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('--write', action='store_true'); args = parser.parse_args()
    frozen = json.dumps(generate(), indent=2, ensure_ascii=False) + '\n'
    if args.write: DEST.write_text(frozen)
    elif not DEST.exists() or DEST.read_text() != frozen: raise SystemExit('edit request vectors differ; review before --write')
