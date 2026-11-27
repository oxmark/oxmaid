# Conformance fixtures

`tests/conformance.rs` compares oxmaid's flowchart parser with Mermaid 12.3
itself. The expected results in `tests/fixtures/flowchart-mermaid-12.3.txt`
come from Mermaid's own `flowDb`, run in Chromium through the full
`mermaid.mermaidAPI.getDiagramFromText` pipeline (so entity encoding and
directives apply, as in a real render).

The inputs are every string passed to `.parse(` in Mermaid's flowchart spec
files (MIT licensed, © Knut Sveidqvist and contributors), plus generated link,
length, keyword and edge-id cases.

## Regenerating

```sh
git clone --depth 1 --branch mermaid@12.3.0 https://github.com/mermaid-js/mermaid /tmp/mermaid
cd tools/conformance
npm install
python3 inputs.py /tmp/mermaid/packages/mermaid/src/diagrams/flowchart/parser inputs.json
node oracle.js inputs.json oracle.json
python3 fixture.py oracle.json ../../tests/fixtures/flowchart-mermaid-12.3.txt
```

To see what differs, run the test with `OXMAID_CONFORMANCE_OUT=/tmp/actual.txt`:
it writes oxmaid's dump there and the normalized expectations next to it
(`/tmp/actual.txt.expected`).
