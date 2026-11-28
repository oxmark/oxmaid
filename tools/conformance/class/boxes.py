# Extracts the node boxes Mermaid 12.3 draws for each class diagram of
# oracle.json, for tests/class_render.rs: each case is `>>> source` (escaped)
# then one line per node, `id <tab> half width <tab> half height` followed by
# the `annotation|label|members|methods` group offsets of a class box.
# Only cases in the default setup are kept (ELK layout, neo look, default
# theme, HTML labels), which is what oxmaid renders.
# usage: python3 boxes.py oracle.json ../../../tests/fixtures/class-boxes-mermaid-12.3.txt
import json, re, sys
d = json.load(open(sys.argv[1]))
def esc(s):
    return s.replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')
SKIP = re.compile(r"htmlLabels: *false|look: *(handDrawn|classic)|layout: *dagre|'theme'|theme:")
def boxes(svg):
    svg = re.sub(r'<style>.*?</style>', '', svg, flags=re.S)
    out = []
    for p in re.split(r'(?=<g class="node )', svg)[1:]:
        m = re.match(r'<g class="node [^"]*" id="d-([^"]*)"', p)
        if not m:
            continue
        sz = re.search(r'<path d="M(-?[\d.]+) (-?[\d.]+) ', p) or \
            re.search(r'<rect[^>]*x="(-?[\d.]+)" y="(-?[\d.]+)"', p)
        groups = re.findall(r'<g class="(\w+)-group text" transform="translate\(([^)]*)\)"', p)
        row = [esc(m.group(1)), '%g' % abs(float(sz.group(1))), '%g' % abs(float(sz.group(2)))]
        row += ['%s=%s' % (g, ','.join('%g' % float(v) for v in t.split(','))) for g, t in groups]
        out.append('\t'.join(row))
    return out
out = []
for c in d:
    if 'svg' not in c or 'classes' not in c or SKIP.search(c['src']):
        continue
    out.append('>>> ' + esc(c['src']))
    out.extend(boxes(c['svg']))
open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
