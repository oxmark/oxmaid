# Converts the Mermaid 12.3 class dump (oracle.json) into the fixture text
# format read by tests/class_conformance.rs.
# usage: python3 fixture.py oracle.json ../../../tests/fixtures/class-mermaid-12.3.txt
import json, re, sys
d = json.load(open(sys.argv[1]))
def esc(s):
    return '' if s is None else str(s).replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')
def lst(v):
    return esc('\x1f'.join(v or []))
def flag(b): return '1' if b else ''
out = []
for c in d:
    if 'other' in c: continue
    out.append('>>> ' + esc(c['src']))
    if 'error' in c:
        out.append('error'); continue
    out.append('dir\t' + c['direction'])
    for k, v in c['classes']:
        # The dom id counter is module state: keep the name part only.
        dom = re.sub(r'-\d+$', '', v['domId'])
        out.append('\t'.join(['class', esc(k), esc(v['id']), esc(v['type']), esc(v['label']), esc(v['text']),
            esc(v['cssClasses']), lst(v['annotations']), lst(v['styles']), esc(dom), esc(v.get('parent')),
            esc(v.get('link')), esc(v.get('linkTarget')), flag(v.get('haveCallback')), esc(v.get('tooltip'))]))
        for kind, ms in (('attr', v['members']), ('meth', v['methods'])):
            for m in ms:
                out.append('\t'.join([kind, esc(m['visibility']), esc(m['classifier']), esc(m.get('id')),
                    esc(m.get('parameters')), esc(m.get('returnType')), esc(m['text'])]))
    for r in c['relations']:
        rel = r['relation']
        out.append('\t'.join(['rel', esc(r['id1']), esc(r['id2']), str(rel['type1']), str(rel['type2']),
            str(rel['lineType']), esc(r['relationTitle1']), esc(r['relationTitle2']), esc(r.get('title'))]))
    for k, n in c['notes']:
        out.append('\t'.join(['note', esc(k), esc(n.get('class')), esc(n['text']), esc(n.get('parent'))]))
    for k, n in c['namespaces']:
        out.append('\t'.join(['ns', esc(k), esc(n['label']), esc(n.get('parent')), flag(n['explicit']),
            lst([x for x, _ in n['classes']]), lst([x for x, _ in n['notes']]), lst([x for x, _ in n['children']])]))
    for k, s in c['styleClasses']:
        out.append('\t'.join(['def', esc(k), lst(s['styles']), lst(s['textStyles'])]))
    for i in c['interfaces']:
        out.append('\t'.join(['iface', esc(i['id']), esc(i['label']), esc(i['classId'])]))
    if c['title'] or c['accTitle'] or c['accDescr']:
        out.append('\t'.join(['meta', esc(c['title']), esc(c['accTitle']), esc(c['accDescr'])]))
open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
