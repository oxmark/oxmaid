// Builds the class conformance inputs: every `.mmd` file of Mermaid's class
// e2e corpus, plus every string expression of the class spec files (literals
// and `'a' + 'b'` concatenations) that holds a `classDiagram`.
// usage: node inputs.js <mermaid checkout> inputs.json
const fs = require('fs'); const path = require('path');
const [root, outFile] = process.argv.slice(2);
const out = []; const seen = new Set();
const add = (file, src) => { if (!seen.has(src)) { seen.add(src); out.push({ file, src }); } };
const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) =>
  e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]);
const e2e = path.join(root, 'e2e/diagrams');
for (const f of walk(path.join(e2e, 'class-diagram')).filter((f) => f.endsWith('.mmd')).sort())
  add(path.relative(e2e, f), fs.readFileSync(f, 'utf8'));
const HEAD = /^\s*(%%\{[\s\S]*?\}%%\s*)*(---\n[\s\S]*?\n---\s*)?classDiagram/;
const specDir = path.join(root, 'packages/mermaid/src/diagrams/class');
for (const f of walk(specDir).filter((f) => /\.spec\.[jt]s$/.test(f)).sort()) {
  const s = fs.readFileSync(f, 'utf8');
  // Right-hand sides of assignments and arguments of parse()/getDiagramFromText().
  const re = /(?:=|parse\(|getDiagramFromText\(|render\(\s*'[^']*',)\s*((?:`(?:[^`\\]|\\.)*`|'(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*"|\s*\+\s*)+)/g;
  for (const m of s.matchAll(re)) {
    const expr = m[1].trim().replace(/\+\s*$/, '');
    let v;
    try { v = new Function('return (' + expr + ')')(); } catch { continue; }
    if (typeof v === 'string' && HEAD.test(v)) add(path.basename(f), v);
  }
}
fs.writeFileSync(outFile, JSON.stringify(out, null, 0));
console.log(out.length);
