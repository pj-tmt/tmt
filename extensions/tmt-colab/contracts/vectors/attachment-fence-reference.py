"""Independent known answers for the message-attachment fence; stdlib only, no product imports."""
import argparse
import hashlib
import json
import struct
from pathlib import Path

DEST = Path(__file__).with_name('attachment-fence-v1.json')

def lp(*fields):
    return b''.join(struct.pack('>I', len(v)) + v for v in fields)
def ident(n):
    return f'00000000-0000-4000-8000-{n:012x}'
SPACE = '4kph3kmtxo7dinlvoixpw642ozfibd2w'

def fence(space, page, epoch, revision, head_hash, author):
    """`v1:` + SHA-256 over a label no page revision shares, so neither can stand in for the other."""
    return 'v1:' + hashlib.sha256(lp(
        b'tmt-colab-attachment-fence-v1', b'1', space.encode(), page.encode(),
        epoch.encode(), revision.encode(), bytes.fromhex(head_hash), author.encode())).hexdigest()

def generate():
    base = dict(space=SPACE, page=ident(2), epoch='1', revision='2', headHash='11' * 32, author=ident(18))
    variants = [
        ('base', {}),
        ('another-space', dict(space='a' * 32)),
        ('another-page', dict(page=ident(3))),
        ('another-epoch', dict(epoch='2')),
        ('another-membership-revision', dict(revision='3')),
        ('another-membership-hash', dict(headHash='22' * 32)),
        ('another-author', dict(author=ident(19))),
    ]
    cases = []
    for name, change in variants:
        v = {**base, **change}
        cases.append(dict(name=name, **v, fence=fence(v['space'], v['page'], v['epoch'], v['revision'], v['headHash'], v['author'])))
    assert len({c['fence'] for c in cases}) == len(cases)
    return dict(version=1, cases=cases)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('--write', action='store_true'); args = parser.parse_args()
    frozen = json.dumps(generate(), indent=2) + '\n'
    if args.write: DEST.write_text(frozen)
    elif not DEST.exists() or DEST.read_text() != frozen: raise SystemExit('attachment fence vectors differ; review before --write')
