# oxmaid

A Rust port of [Mermaid](https://mermaid.js.org), the text-to-diagram library.
oxmaid turns Mermaid source into SVG without a browser, a DOM or JavaScript.

It is part of the Oxmark project (a Rust port of Comark). The core aims for
raw performance and an idiomatic Rust API; bindings for other languages
(such as PHP) will live in separate projects.

> **Status: early.** Only flowcharts are ported. oxmaid targets
> **Mermaid 12.3**: the parser matches Mermaid's own results on 440 reference
> cases, and the output follows Mermaid 12's default look (`neo` look,
> `redux-color` theme, ELK layout). The layout approximates ELK, so complex
> diagrams are close to Mermaid's but not identical.

## Usage

```rust
let svg = oxmaid::render(r#"
flowchart LR
    A[Start] --> B{Is it working?}
    B -->|Yes| C[Ship it]
    B -->|No| D[Debug] --> B
"#)?;
```

- `oxmaid::render(src)` / `oxmaid::render_with(src, &RenderOptions)` return an SVG string.
- `oxmaid::parse(src)` returns the typed diagram (`Diagram::Flowchart`) for tools that need the model.
- `oxmaid::detect(src)` returns the diagram kind from the header.
- Errors carry a kind, a line and a column (`Error::kind()`, `line()`, `column()`, `message()`).
- Set `RenderOptions::id_prefix` when several diagrams share one HTML page.

Try it from the command line:

```sh
cargo run --example render -- examples/diagrams/basic.mmd > basic.svg
```

## Supported flowchart syntax

| Feature | Syntax |
| --- | --- |
| Header and direction | `flowchart TB`, `graph LR`, `TD`, `BT`, `RL` |
| Shapes | `[ ]`, `( )`, `([ ])`, `[[ ]]`, `[( )]`, `(( ))`, `((( )))`, `{ }`, `{{ }}`, `> ]`, `[/ /]`, `[\ \]`, `[/ \]`, `[\ /]` |
| Labels | plain or `"quoted"`, `<br>` line breaks, `#quot;` / `#35;` entities |
| Links | `-->`, `---`, `-.->`, `-.-`, `==>`, `===`, `~~~`, `<-->`, `--o`, `--x`, longer forms (`--->`) |
| Link text | `-->\|text\|`, `-- text -->`, `-. text .->`, `== text ==>` |
| Chaining | `A --> B --> C`, `A & B --> C & D` |
| Subgraphs | `subgraph id [title] ... end`, nesting, links to a subgraph |
| Styling | `classDef`, `class`, `:::class`, `style` (`fill`, `stroke`, `stroke-width`, `stroke-dasharray`, `color`) |
| Other | `;` separators, `%%` comments, YAML front matter (skipped) |

The parser follows Mermaid 12.3's grammar and `flowDb`, including `linkStyle`,
`click`, `accTitle` / `accDescr`, Markdown strings, Font Awesome icons, edge
ids and `@{ ... }` data (`shape`, `label`, `animate`, `curve`), and the same
quirks (entity handling, ids such as `a-b`, keyword boundaries). See
`tests/conformance.rs` and `tools/conformance/`.

Parsed but not drawn yet: subgraph `direction` and the v12 shapes added with
`@{ shape: ... }` (drawn as rectangles).

## Looks and themes

Mermaid 12 renders flowcharts with the `neo` look, the `redux-color` theme and
the ELK layout by default, and so does oxmaid: same stylesheet, markers, shape
geometry, Arial text metrics and spacing. The previous look is available with
`look: classic` in the front matter:

```
---
config:
  look: classic
---
flowchart LR
  A --> B
```

Other themes (`dark`, `forest`, ...) are not ported yet.

## Architecture

```
src/
  lib.rs           public API: detect, parse, render, Diagram, Error
  source.rs        shared line splitting (front matter, comments)
  flowchart/
    mod.rs         flowchart model (nodes, edges, subgraphs, classes)
    parser.rs      hand-written parser with line/column errors
    neo.rs         default renderer (`neo` look, `redux-color` theme)
    neo.css        Mermaid's stylesheet for that look
    render.rs      `classic` look renderer
  layout.rs        layered (Sugiyama) layout, a stand-in for ELK and dagre
  text.rs          text measurement
  font_metrics.rs  Arial advance widths and kerning
  svg.rs           escaping and number formatting
```

Each diagram type gets its own module with a model, a parser and a renderer.
Layout and text measurement are shared.

- **Parser.** Mermaid uses Jison grammars. oxmaid uses hand-written parsers
  that follow the same syntax and report precise positions.
- **Layout.** Mermaid 12 uses ELK (layered). `layout.rs` implements the same
  family of algorithm: cycle breaking, ranking, dummy nodes for long edges and
  edge labels, barycenter ordering, and, for the neo look, balanced
  Brandes-Köpf placement that lines links up port to port, with ELK's spacing
  and orthogonal routing. Links to a subgraph keep the whole subgraph past the
  other end. Crossing minimization and cycle breaking differ from ELK, so node
  order can differ on graphs with several valid layouts. A full ELK port may
  follow. The classic look keeps a dagre-like placement.
- **Text.** Mermaid measures text in the browser. oxmaid uses Arial
  (Liberation Sans) advance widths and kerning, which is what Mermaid measures
  with on the default font stack.
- **SVG.** The neo renderer writes the same SVG structure as Mermaid
  (`foreignObject` labels, the same ids and classes). The classic renderer
  inlines presentation attributes, so its output also works in non-browser
  renderers. User styles are filtered to safe values in both.

## Roadmap

1. Flowchart parity: a full ELK port, the v12 `@{ shape }` shapes, subgraph
   `direction`, the other themes.
2. Next diagram types, likely sequence, state and class diagrams.
3. Performance work: benchmarks against Mermaid, byte-level parsing,
   faster crossing counts for large graphs.

## Contributing

Commits follow [Conventional Commits](https://www.conventionalcommits.org)
(`feat:`, `fix:`, `docs:`, ...). Releases and the changelog are produced by
[release-please](https://github.com/googleapis/release-please) from those
commit messages.

```sh
cargo fmt --check && cargo clippy --all-targets && cargo test
```

## License

Apache-2.0. Mermaid itself is MIT licensed; oxmaid is an independent
reimplementation. The neo stylesheet (`src/flowchart/neo.css`) and arrow
markers (`src/flowchart/neo_markers.svg`) are adapted from Mermaid's output,
© Knut Sveidqvist and contributors, under the MIT license.
