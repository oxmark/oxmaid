// Dumps Mermaid 12.3 classDb state and rendered SVGs as JSON.
// usage: node class/oracle.js inputs.json oracle.json   (from tools/conformance)
const { chromium } = require('playwright'); const fs = require('fs'); const path = require('path');
(async () => {
  const [inp, out] = process.argv.slice(2);
  const cases = JSON.parse(fs.readFileSync(inp, 'utf8'));
  const b = await chromium.launch(); const p = await b.newPage();
  const res = [];
  for (const c of cases) {
    // A fresh page per case: Mermaid keeps module state between renders.
    await p.setContent('<html><body><div id=c></div></body></html>');
    await p.addScriptTag({ path: path.join(__dirname, '../node_modules/mermaid/dist/mermaid.min.js') });
    await p.evaluate(() => mermaid.initialize({ startOnLoad: false }));
    const r = await p.evaluate(async (src) => {
      const out = {};
      // Render first: it applies this diagram's directives to the global
      // configuration, which the database reads.
      try {
        out.svg = (await mermaid.render('d', src)).svg;
      } catch (e) { out.renderError = String(e.message || e).slice(0, 300); }
      document.querySelectorAll('body > :not(#c)').forEach((n) => n.remove());
      const plain = (v) => JSON.parse(JSON.stringify(v, (k, x) =>
        x instanceof Map ? [...x.entries()] : x instanceof Set ? [...x] : typeof x === 'function' ? undefined : x));
      try {
        const d = await mermaid.mermaidAPI.getDiagramFromText(src);
        const db = d.db;
        if (!db.getClasses) return { other: d.type };
        out.direction = db.getDirection();
        out.classes = plain(db.getClasses());
        out.relations = plain(db.getRelations());
        out.notes = plain(db.getNotes());
        out.namespaces = plain(db.getNamespaces());
        out.styleClasses = plain(db.styleClasses);
        out.interfaces = plain(db.interfaces);
        out.title = db.getDiagramTitle() || null;
        out.accTitle = db.getAccTitle() || null;
        out.accDescr = db.getAccDescription() || null;
      } catch (e) { return { ...out, error: String(e.message || e).slice(0, 300) }; }
      return out;
    }, c.src);
    res.push({ src: c.src, file: c.file, ...r });
  }
  fs.writeFileSync(out, JSON.stringify(res, null, 1));
  await b.close();
})();
