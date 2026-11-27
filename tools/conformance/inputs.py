# Builds the conformance inputs: every string passed to `.parse(` in the
# Mermaid flowchart spec files, plus generated link, length and keyword cases.
# usage: python3 inputs.py <mermaid>/packages/mermaid/src/diagrams/flowchart/parser inputs.json
import re, json, glob, sys
out=[]; seen=set()
for f in sorted(glob.glob(sys.argv[1]+'/*.spec.js')):
    s=open(f).read()
    # string literal argument of .parse(
    for m in re.finditer(r"\.parse\(\s*(?:\n\s*)?('(?:[^'\\\n]|\\.)*'|\"(?:[^\"\\\n]|\\.)*\"|`(?:[^`\\$]|\\.)*`)\s*\)", s):
        lit=m.group(1); q=lit[0]; body=lit[1:-1]
        if q=='`':
            src=body.replace('\\`','`').replace('\\\\','\\')
        else:
            try: src=json.loads('"'+body.replace('\\'+q,q).replace('"','\\"').replace('\\\\"','\\"')+'"') if q=="'" else json.loads(lit)
            except Exception: continue
        if src in seen: continue
        seen.add(src); out.append({'file': f.split('/')[-1], 'src': src})


d=out
def add(s):
    if s not in seen: seen.add(s); d.append({'file':'generated','src':s})
for st,en in [('--','-->'),('==','==>'),('-.','.->'),('<--','-->'),('<==','==>'),('<-.','.->'),('o--','--o'),('x--','--x'),('--','--o'),('--','--x'),('==','==o'),('-.','.-x')]:
    add(f"flowchart TD\nA {st} text {en} B")
for a in ['-->','---','==>','===','-.->','-.-','<-->','o--o','x--x','--o','--x','~~~','<==>','<-.->','x==x','o-.-o']:
    add(f"graph TD\nA{a}B"); add(f"graph TD\nA {a}|lbl| B")
for n in range(1,13):
    add(f"graph TD\nA -{'-'*n}> B"); add(f"graph TD\nA ={'='*n}> B"); add(f"graph TD\nA -{'.'*n}-> B"); add(f"graph TD\nA -- t -{'-'*n}> B")
for kw in ['graph','flowchart','flowchart-elk','subgraph','end','style','linkStyle','interpolate','classDef','class','href','call','click','_self','_blank','_parent','_top','direction','default','TB','LR']:
    for t in ['{k}.node','{k}-node','{k}/node','node.{k}','node-{k}']:
        add(f"graph TD\n{t.format(k=kw)}-->B")
for s in ['A[bla]e1@--> B','A@{shape:circle}e1@--> B','A@{shape:circle} e1@--> B@{shape:circle} e2@--> C','A e[1]@--> B','A e1@-->B\ne1@{animate: true}','A e1@-->B\ne1@{curve: basis}','A-->B & C\nB-->D','A@{ shape: docs, label: "x" }','A@{ shape: notvalid }','A@{ label: "**b**" }','A:::c1 --> B:::c2','A["`**md**`"]','A --> B\nlinkStyle 0 stroke:red\nlinkStyle default stroke:blue','A --> B\nlinkStyle 3 stroke:red','A(-ell-)','A>odd]','A["a\\nb"]','A[a\\nb]','A[a<br/>b<br >c]','A --> B\nclick A href "http://x" "tip" _blank','A --> B\nclick B call cb()','subgraph "a b"\nx\nend','subgraph a\nx\nend\nsubgraph a\ny\nend','subgraph s1\nA\nend\nsubgraph s2\nA\nend','A-->B;B-->C','A --> B --> C & D','accTitle: t\naccDescr: d\nA','accDescr {\nmulti\nline\n}\nA','A &amp; B','A[#quot;x#quot;]','A --text--> B','A-- text with - dash -->B','A -->|a| B -->|b| C','A---|x|B','A==text==B','A-. text .-B']:
    add("flowchart LR\n"+s)
json.dump(d, open(sys.argv[2], 'w'), indent=0)
print(len(d))
