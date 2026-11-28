# Converts the Mermaid 12.3 sequence dump (oracle.json) into the fixture text
# format read by tests/sequence_conformance.rs.
# usage: python3 fixture.py oracle.json ../../../tests/fixtures/sequence-mermaid-12.3.txt
import json, sys
d = json.load(open(sys.argv[1]))
def esc(s):
    return '' if s is None else str(s).replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')
def num(v):
    if v is None: return ''
    if isinstance(v, float) and v.is_integer(): v = int(v)
    return str(v)
def flag(b): return '1' if b else ''
PLACEMENT = {0: 'left', 1: 'right', 2: 'over'}
out = []
for c in d:
    if 'other' in c: continue
    out.append('>>> ' + esc(c['src']))
    if 'error' in c:
        out.append('error'); continue
    for a in c['actors']:
        props = dict(a['properties'])
        out.append('\t'.join(['actor', esc(a['id']), esc(a['name']), esc(a['description']), flag(a['wrap']), esc(a['type']),
            num(a['box']), esc(a['prev']), esc(a['next']), esc(';'.join(f'{k}={v}' for k, v in a['links'])),
            esc(props.get('class')), esc(props.get('icon'))]))
    for m in c['messages']:
        if isinstance(m['message'], dict):
            a = m['message']
            out.append('\t'.join(['auto', num(a['start']), num(a['step']), flag(a['visible'])]))
            continue
        out.append('\t'.join(['msg', str(m['type']), esc(m['from']), esc(m['to']), esc(m['message']), flag(m['wrap']),
            flag(m['activate']), PLACEMENT.get(m['placement'], ''), num(m['central'] or None)]))
    for b in c['boxes']:
        out.append('\t'.join(['box', esc(b['name']), esc(b['fill']), flag(b['wrap']), esc(','.join(b['actors']))]))
    for k, v in c['created']: out.append(f'created\t{esc(k)}\t{v}')
    for k, v in c['destroyed']: out.append(f'destroyed\t{esc(k)}\t{v}')
    if c['title'] or c['accTitle'] or c['accDescr']:
        out.append('\t'.join(['meta', esc(c['title']), esc(c['accTitle']), esc(c['accDescr'])]))
open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
