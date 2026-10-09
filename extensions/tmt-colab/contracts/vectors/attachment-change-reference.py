"""Independent known answers for the typed document attachment change; stdlib only.

The oracle re-implements the change semantics from the contract text, imports no product
code and writes `attachment-change-v1.json`. Run without `--write` to verify the frozen file."""
import argparse
import copy
import hashlib
import json
from pathlib import Path

DEST = Path(__file__).with_name('attachment-change-v1.json')
CAP = 128
SOURCE = '<h1>Files</h1><p>Media lives outside the document.</p>'
OTHER = '<p>A different source</p>'


def ident(n):
    return f'00000000-0000-4000-8000-{n:012x}'


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def descriptor(n, digest, **over):
    base = {
        'version': 1,
        'attachmentId': ident(n),
        'space': '4kph3kmtxo7dinlvoixpw642ozfibd2w',
        'page': ident(2),
        'epoch': '1',
        'namespace': 'content',
        'objectId': f'{n & 0xff:02x}' * 32,
        'authorDevice': ident(0x12),
        'membershipRevision': '2',
        'source': {'kind': 'document', 'sourceDigest': digest},
        'envelopeHash': '34' * 32,
        'signature': 'mTOaOQ2Qzt_WcbjBQQt2iUqkblc1b2FAk206Z0JlpWnOrVJLxRZU46fBAScdhWr9MWLORFA6BOiYXOphXcbyBA',
        'payloadSha256': '30' * 32,
        'payloadBytes': '631',
        'plaintextBytes': '21',
        'filename': f'file-{n}.bin',
        'mediaType': 'application/octet-stream',
    }
    base.update(over)
    return base


def fill(count):
    """Distinct stand-ins for a long list: ids 0x1000 + i, same shape as `a`."""
    return [descriptor(0x1000 + i, sha(SOURCE)) for i in range(count)]


def apply(current, change):
    """The contract: removals (each must exist), then additions (identical repeat is a no-op);
    a source binding, namespace or uniqueness violation, and a result over 128, are refused."""
    digest = sha(SOURCE)
    ids = set()
    for d in change.get('set', []):
        if d['namespace'] != 'content' or d['source'] != {'kind': 'document', 'sourceDigest': digest}:
            return None
        if d['attachmentId'] in ids:
            return None
        ids.add(d['attachmentId'])
    for i in change.get('remove', []):
        if i in ids:
            return None
        ids.add(i)
    out = copy.deepcopy(current)
    for i in change.get('remove', []):
        keep = [d for d in out if d['attachmentId'] != i]
        if len(keep) + 1 != len(out):
            return None
        out = keep
    for d in change.get('set', []):
        found = [e for e in out if e['attachmentId'] == d['attachmentId']]
        if found:
            if found[0] != d:
                return None
        else:
            out.append(copy.deepcopy(d))
    return out if len(out) <= CAP else None


def build():
    digest = sha(SOURCE)
    d = {
        'a': descriptor(0xA1, digest),
        'b': descriptor(0xB2, digest),
        'changedA': descriptor(0xA1, digest, filename='renamed.bin'),
        'foreignDigest': descriptor(0xC3, sha(OTHER)),
        'ownNamespace': descriptor(
            0xD4,
            digest,
            namespace='own',
            source={
                'kind': 'message',
                'writerId': ident(0x12),
                'messageId': ident(0x77),
                'messageRevision': '1',
            },
        ),
    }
    a, b = d['a']['attachmentId'], d['b']['attachmentId']

    def resolve(items):
        if isinstance(items, dict):
            return fill(items['generated'])
        return [d[k] for k in items]

    def case(name, current, set_=(), remove=()):
        """`current` and `set` name entries of `descriptors` (or `{"generated": n}` for the
        deterministic fillers `descriptor(0x1000 + i)`); results list attachment IDs in order."""
        change = {}
        if set_:
            change['set'] = [d[k] for k in set_]
        if remove:
            change['remove'] = list(remove)
        out = apply(resolve(current), change)
        return {
            'name': name,
            'current': current,
            'set': list(set_),
            'remove': list(remove),
            'result': 'refused' if out is None else [x['attachmentId'] for x in out],
        }

    cases = [
        case('set-one', [], ['a']),
        case('set-two-keeps-order', [], ['a', 'b']),
        case('append-to-existing', ['a'], ['b']),
        case('identical-repeat-is-a-noop', ['a'], ['a']),
        case('different-descriptor-under-an-existing-id', ['a'], ['changedA']),
        case('remove-one', ['a', 'b'], [], [a]),
        case('remove-all', ['a'], [], [a]),
        case('remove-absent', ['a'], [], [b]),
        case('set-and-remove-the-same-id', ['a'], ['b'], [b]),
        case('remove-then-add-another', ['a'], ['b'], [a]),
        case('foreign-source-digest', [], ['foreignDigest']),
        case('own-namespace-message-descriptor', [], ['ownNamespace']),
        case('duplicate-in-set', [], ['a', 'a']),
        case('duplicate-in-remove', ['a'], [], [a, a]),
        case('cap-reached-then-one-more', {'generated': CAP}, ['a']),
        case('cap-reached-swap-one', {'generated': CAP}, ['a'], [fill(1)[0]['attachmentId']]),
        case('empty-change', ['a']),
    ]
    return {
        'version': 1,
        'source': SOURCE,
        'sourceSha256': digest,
        'cap': CAP,
        'descriptors': d,
        'cases': cases,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    text = json.dumps(build(), ensure_ascii=False, indent=1, sort_keys=False) + '\n'
    if args.write:
        DEST.write_text(text)
    elif DEST.read_text() != text:
        raise SystemExit('attachment-change-v1.json differs from the oracle')
