// Dumps Mermaid 12.3 flowDb results as JSON. usage: node oracle.js inputs.json oracle.json
const { chromium } = require('playwright'); const fs = require('fs');
(async () => {
  const [inp, out] = process.argv.slice(2);
  const cases = JSON.parse(fs.readFileSync(inp, 'utf8'));
  const b = await chromium.launch(); const p = await b.newPage();
  await p.setContent('<html><body></body></html>');
  await p.addScriptTag({ path: require.resolve('mermaid/dist/mermaid.min.js') });
  await p.evaluate(() => mermaid.initialize({ startOnLoad: false }));
  const res = [];
  for (const c of cases) {
    const r = await p.evaluate(async (src) => {
      try {
        const d = await mermaid.mermaidAPI.getDiagramFromText(src);
        const db = d.db;
        if (!db.getVertices) return { other: d.type };
        const verts = [...db.getVertices().values()].map(v => ({ id: v.id, label: v.text ?? null, labelType: v.labelType, shape: v.type ?? null, classes: [...(v.classes||[])], styles: v.styles||[], link: v.link ?? null, target: v.linkTarget ?? null }));
        const edges = db.getEdges().map(e => ({ id: e.id, from: e.start, to: e.end, type: e.type, stroke: e.stroke, length: e.length, text: e.text, labelType: e.labelType, style: e.style||[], interpolate: e.interpolate ?? null, animate: e.animate ?? null }));
        const subs = db.getSubGraphs().map(s => ({ id: s.id, title: s.title, nodes: s.nodes, dir: s.dir ?? null, classes: s.classes||[] }));
        return { dir: db.getDirection(), verts, edges, subs, accTitle: db.getAccTitle(), accDescr: db.getAccDescription(), classes: [...db.getClasses().entries()].map(([k,v]) => [k, v.styles]) };
      } catch (e) { return { error: String(e.message || e).slice(0, 300) }; }
    }, c.src);
    res.push({ src: c.src, file: c.file, ...r });
  }
  fs.writeFileSync(out, JSON.stringify(res, null, 1));
  await b.close();
})();
