# Builds the sequence conformance inputs: every `.mmd` file of Mermaid's
# sequence e2e corpus, plus every string literal of the sequence spec files
# that holds a `sequenceDiagram`.
# usage: python3 inputs.py <mermaid checkout> inputs.json
import re, json, glob, sys, os
root = sys.argv[1]
out = []; seen = set()
def add(file, src):
    if src in seen: return
    seen.add(src); out.append({'file': file, 'src': src})
for f in sorted(glob.glob(root + '/e2e/diagrams/sequence/**/*.mmd', recursive=True)):
    add(os.path.relpath(f, root + '/e2e/diagrams'), open(f).read())
lit = re.compile(r"`((?:[^`\\$]|\\.|\$(?!\{))*)`|'((?:[^'\\\n]|\\.)*)'|\"((?:[^\"\\\n]|\\.)*)\"")
for f in sorted(glob.glob(root + '/packages/mermaid/src/diagrams/sequence/*.spec.*')):
    s = open(f).read()
    for m in lit.finditer(s):
        if m.group(1) is not None:
            src = re.sub(r'\\(.)', lambda e: {'n': '\n', 't': '\t'}.get(e.group(1), e.group(1)), m.group(1))
        else:
            body = m.group(2) if m.group(2) is not None else m.group(3)
            src = re.sub(r'\\(.)', lambda e: {'n': '\n', 't': '\t'}.get(e.group(1), e.group(1)), body)
        if not re.match(r'\s*(%%\{.*?\}%%\s*)*(---\n[\s\S]*?\n---\s*)?sequenceDiagram', src): continue
        add(os.path.basename(f), src)
json.dump(out, open(sys.argv[2], 'w'), indent=0)
print(len(out))
