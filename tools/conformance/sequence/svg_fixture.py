# Writes the SVG fixture read by tests/sequence_svg.rs from the Mermaid 12.3
# dump (oracle.json): for each diagram drawn with the default theme, its source
# and the FNV-1a 64 hash and length of the SVG Mermaid rendered with id "d".
# The SVGs themselves stay out of the repository (about 1.6 MB); regenerate
# oracle.json to diff one.
# usage: python3 svg_fixture.py oracle.json ../../../tests/fixtures/sequence-svg-mermaid-12.3.txt
import json, re, sys

def esc(s):
    return s.replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')

def fnv1a64(b):
    h = 0xcbf29ce484222325
    for x in b:
        h = ((h ^ x) * 0x100000001b3) & 0xffffffffffffffff
    return h

THEME = re.compile(r"theme['\"]?\s*:\s*['\"]?([\w-]+)")
out = []
for c in json.load(open(sys.argv[1])):
    svg = c.get('svg')
    theme = THEME.search(c['src'])
    if not svg or (theme and theme.group(1) != 'redux-color'):
        continue
    b = svg.encode()
    out.append('>>> ' + esc(c['src']))
    out.append(f'{fnv1a64(b):016x}\t{len(b)}')
open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
