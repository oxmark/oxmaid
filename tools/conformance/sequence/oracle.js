// Dumps Mermaid 12.3 sequenceDb results and rendered SVGs as JSON.
// usage: node oracle.js inputs.json oracle.json   (run with NODE_PATH=../node_modules)
const { chromium } = require('playwright'); const fs = require('fs'); const path = require('path');
(async () => {
  const [inp, out] = process.argv.slice(2);
  const cases = JSON.parse(fs.readFileSync(inp, 'utf8'));
  const b = await chromium.launch(); const p = await b.newPage();
  const res = [];
  for (const c of cases) {
    // A fresh page per case: Mermaid keeps module state (the participant
    // counter behind `actor<N>` ids, the site configuration) between renders.
    await p.setContent('<html><body><div id=c></div></body></html>');
    await p.addScriptTag({ path: path.join(__dirname, '../node_modules/mermaid/dist/mermaid.min.js') });
    await p.evaluate(() => mermaid.initialize({ startOnLoad: false }));
    const r = await p.evaluate(async (src) => {
      const out = {};
      // Render first: it resets the configuration and applies this diagram's
      // directives, which the database reads when it is created.
      try {
        const r = await mermaid.render('d', src);
        out.svg = r.svg;
      } catch (e) { out.renderError = String(e.message || e).slice(0, 300); }
      document.querySelectorAll('body > :not(#c)').forEach(n => n.remove());
      try {
        const d = await mermaid.mermaidAPI.getDiagramFromText(src);
        const db = d.db;
        if (!db.getActors) return { other: d.type };
        const boxes = db.getBoxes();
        out.actors = [...db.getActors().entries()].map(([id, a]) => ({
          id, name: a.name, description: a.description, wrap: a.wrap, type: a.type,
          box: a.box ? boxes.indexOf(a.box) : null, prev: a.prevActor ?? null, next: a.nextActor ?? null,
          links: Object.entries(a.links || {}), properties: Object.entries(a.properties || {}),
        }));
        out.messages = db.getMessages().map(m => ({
          from: m.from ?? null, to: m.to ?? null,
          message: typeof m.message === 'object' && m.message !== null ? { start: m.message.start ?? null, step: m.message.step ?? null, visible: m.message.visible } : (m.message ?? null),
          wrap: m.wrap, type: m.type ?? null, activate: m.activate ?? null, placement: m.placement ?? null,
          central: m.centralConnection ?? null,
        }));
        out.boxes = boxes.map(x => ({ name: x.name ?? null, fill: x.fill, wrap: x.wrap, actors: x.actorKeys }));
        out.created = [...db.getCreatedActors().entries()];
        out.destroyed = [...db.getDestroyedActors().entries()];
        out.title = db.getDiagramTitle() || null;
        out.accTitle = db.getAccTitle() || null;
        out.accDescr = db.getAccDescription() || null;
      } catch (e) { return { error: String(e.message || e).slice(0, 300) }; }
      return out;
    }, c.src);
    res.push({ src: c.src, file: c.file, ...r });
  }
  fs.writeFileSync(out, JSON.stringify(res, null, 1));
  await b.close();
})();
