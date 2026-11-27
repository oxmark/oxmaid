# Converts the Mermaid 12.3 flowDb dump (oracle.json) into the fixture text format read by tests/conformance.rs.
import json, re, html, sys
d = json.load(open(sys.argv[1]))
def esc(s):
    return '' if s is None else str(s).replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')
def ent(s):
    s = re.sub(r'ﬂ°°(\d+)¶ß', lambda m: '&#%s;' % m.group(1), s)
    s = re.sub(r'ﬂ°(\w+)¶ß', lambda m: '&%s;' % m.group(1), s)
    return s
def label(s, ty):
    s = ent(s or '')
    s = re.sub(r'<br\s*/?>', '\n', s, flags=re.I)
    if ty != 'markdown':
        s = s.replace('\\n', '\n')
    s = html.unescape(s)
    if ty == 'markdown':
        s = '\n'.join(l.lstrip() for l in s.split('\n'))
    return s.strip()
def css(lst):
    return ';'.join(re.sub(r'\s+', '', ent(x)) for x in lst if x.strip())
out = []
for c in d:
    if 'other' in c: continue
    out.append('>>> ' + esc(c['src']))
    if 'error' in c:
        out.append('error'); continue
    subids = {s['id'] for s in c['subs']}
    out.append('dir\t' + {'TD': 'TB'}.get(c['dir'], c['dir'] or 'TB'))
    for v in c['verts']:
        if v['id'] in subids: continue
        out.append('\t'.join(['node', esc(v['id']), esc(label(v['label'], v['labelType'])), v['labelType'] or 'text', v['shape'] or 'rect', ','.join(v['classes']), esc(css(v['styles'])), esc(v['link']), esc(v['target'])]))
    for e in c['edges']:
        out.append('\t'.join(['edge', esc(e['id']), esc(e['from']), esc(e['to']), e['type'], e['stroke'], str(e['length']), esc(label(e['text'], e['labelType'])), e['labelType'] or 'text', esc(css(e['style'])), esc(e['interpolate']), 'animate' if e['animate'] else '']))
    for s in c['subs']:
        out.append('\t'.join(['sub', esc(s['id']), esc(label(s['title'], 'text')), ','.join(sorted(set(s['nodes']))), {'TD': 'TB'}.get(s['dir'], s['dir'] or ''), ','.join(s['classes'])]))
    if c['accTitle'] or c['accDescr']:
        out.append('\t'.join(['acc', esc(c['accTitle']), esc(c['accDescr'])]))
    for k, st in c['classes']:
        out.append('\t'.join(['classdef', esc(k), esc(css(st))]))
open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
