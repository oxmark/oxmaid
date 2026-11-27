# oxmaid

A Rust port of [Mermaid](https://mermaid.js.org), the text-to-diagram library.
oxmaid turns Mermaid source into SVG without a browser, a DOM or JavaScript.

It is part of the Oxmark project (a Rust port of Comark). The Rust core is
meant to be wrapped by extensions for other languages, PHP first, so the
public API is deliberately small and flat.

> **Status: early.** Only flowcharts are ported. The output is close to
> Mermaid's default theme, but not pixel identical.

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

Parsed but not used yet: `direction` inside subgraphs. Ignored: `linkStyle`,
`click`, `accTitle`, `accDescr`, `%%{init}%%` directives, Markdown strings,
Font Awesome icons and the new `@{ shape: ... }` syntax.

## Architecture

```
src/
  lib.rs           public API: detect, parse, render, Diagram, Error
  source.rs        shared line splitting (front matter, comments)
  flowchart/
    mod.rs         flowchart model (nodes, edges, subgraphs, classes)
    parser.rs      hand-written parser with line/column errors
    render.rs      sizing, layout call, SVG output
  layout.rs        layered (Sugiyama) layout, a stand-in for dagre
  text.rs          text measurement (estimated glyph widths)
  svg.rs           escaping and number formatting
```

Each diagram type gets its own module with a model, a parser and a renderer.
Layout and text measurement are shared.

- **Parser.** Mermaid uses Jison grammars. oxmaid uses hand-written parsers
  that follow the same syntax and report precise positions.
- **Layout.** Mermaid uses dagre. `layout.rs` implements the same family of
  algorithm: cycle breaking, ranking, dummy nodes for long edges and edge
  labels, barycenter ordering, and horizontal placement by weighted isotonic
  regression. Clusters (subgraphs) are kept contiguous and never overlap.
  Results are similar to dagre, not identical.
- **Text.** Mermaid measures text in the browser. oxmaid estimates widths
  from a table of average glyph widths. Real font metrics can come later.
- **SVG.** Presentation attributes are inlined (no `<style>` block), so the
  output also works in non-browser renderers. User styles are filtered to
  safe values.

## Roadmap

1. Flowchart parity: Markdown strings, `@{ shape }` syntax, `linkStyle`,
   `click`, subgraph `direction`, themes and `%%{init}%%` configuration.
2. Compare against Mermaid's own test fixtures.
3. Next diagram types, likely sequence, state and class diagrams.
4. Bindings: a C ABI crate, then a PHP extension.

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
reimplementation and does not include Mermaid code.
