// Measures glyph advances (U+0020..U+024F) and ASCII kerning pairs for a CSS
// font family in Chromium, in units of a 2048 em. usage: node measure.js family out.json
const { chromium } = require('playwright'); const fs = require('fs');
(async () => {
  const [family, out] = process.argv.slice(2);
  const b = await chromium.launch(); const p = await b.newPage();
  await p.setContent('<html><body></body></html>');
  const r = await p.evaluate((family) => {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); document.body.appendChild(svg);
    const t = document.createElementNS('http://www.w3.org/2000/svg', 'text'); svg.appendChild(t);
    t.style.fontFamily = family; t.style.fontSize = '2048px';
    const w = (s) => { t.textContent = s; return t.getComputedTextLength(); };
    const adv = [];
    for (let c = 32; c < 0x250; c++) adv.push(Math.round(w('|' + String.fromCodePoint(c) + '|') - w('||')));
    const kern = [];
    for (let a = 32; a < 127; a++) for (let c = 32; c < 127; c++) {
      const d = Math.round(w(String.fromCharCode(a, c)) - adv[a - 32] - adv[c - 32]);
      if (d !== 0) kern.push([a, c, d]);
    }
    return { adv, kern };
  }, family);
  fs.writeFileSync(out, JSON.stringify(r));
  await b.close();
})();
