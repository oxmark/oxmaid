# oxmaid

A Rust port of [Mermaid](https://mermaid.js.org), the text-to-diagram library.
oxmaid turns Mermaid source into SVG without a browser, a DOM or JavaScript.

It is part of the Oxmark project (a Rust port of Comark). The core aims for
raw performance and an idiomatic Rust API; bindings for other languages
(such as PHP) will live in separate projects.

> **Status: early.** Flowcharts, sequence diagrams and class diagrams are
> ported. oxmaid
> targets **Mermaid 12.3** and its default look (`neo` look, `redux-color`
> theme).
>
> - Flowcharts: the parser matches Mermaid's own results on 440 reference
>   cases. The layout approximates ELK, so complex diagrams are close to
>   Mermaid's but not identical.
> - Sequence diagrams: the parser matches Mermaid's `sequenceDb` on 270
>   reference cases, and 197 of the 201 diagrams that Mermaid draws with the
>   default theme come out byte for byte identical to Mermaid's SVG.
> - Class diagrams: the parser matches Mermaid's `classDb` on 416 reference
>   cases, and every class, note and interface box of the 204 diagrams drawn
>   in the default setup has Mermaid's exact size and inner layout. Placement
>   uses the same ELK stand-in as flowcharts.

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
- `oxmaid::parse(src)` returns the typed diagram (`Diagram::Flowchart`, `Diagram::Sequence`, `Diagram::Class`) for tools that need the model.
- `oxmaid::detect(src)` returns the diagram kind from the header.
- Errors carry a kind, a line and a column (`Error::kind()`, `line()`, `column()`, `message()`).
- Set `RenderOptions::id_prefix` when several diagrams share one HTML page.

Try it from the command line:

```sh
cargo run --example render -- examples/diagrams/basic.mmd > basic.svg
cargo run --example render -- examples/diagrams/sequence.mmd > sequence.svg
cargo run --example render -- examples/diagrams/class.mmd > class.svg
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

## Supported sequence diagram syntax

The whole Mermaid 12.3 grammar and `sequenceDb`:

| Feature | Syntax |
| --- | --- |
| Participants | `participant`, `actor`, `as` aliases, `create` / `destroy`, `@{ "type": ..., "alias": ... }` with the `boundary`, `control`, `entity`, `database`, `collections` and `queue` types |
| Messages | `->`, `-->`, `->>`, `-->>`, `<<->>`, `<<-->>`, `-x`, `--x`, `-)`, `--)`, central connections `()->>()`, `+` / `-` activation shorthands |
| Notes | `note left of`, `right of`, `over A,B`, `:wrap:` / `:nowrap:` |
| Blocks | `loop`, `alt` / `else`, `opt`, `par` / `and`, `par_over`, `critical` / `option`, `break`, `rect` |
| Other | `activate` / `deactivate`, `autonumber` (start, step, `off`), `box`, `links` / `link`, `properties`, `title`, `accTitle` / `accDescr`, `%%{init}%%` directives and front matter config |

The renderer is a port of Mermaid's `sequenceRenderer` and `svgDraw`: the same
SVG structure, ids, classes, markers, layout arithmetic and text metrics
(Mermaid measures sequence labels in Times New Roman, which oxmaid
reproduces glyph by glyph, ink bounds included). Known differences: the
`themeCSS` option is ignored, and classic-look database labels miss the
float32 rounding noise of Chromium's bounding boxes (`229.00000381469727`
instead of `229`). See `tests/sequence_svg.rs` and
`tools/conformance/sequence/`.

## Supported class diagram syntax

The whole Mermaid 12.3 grammar and `classDb`, run from Mermaid's own parse
tables:

| Feature | Syntax |
| --- | --- |
| Classes | `class A`, `class A["label"]`, `` class `A B` ``, generics `A~T~`, bodies `{ ... }`, `A : +member` |
| Members | visibility `+ - # ~`, classifiers `$` (underlined) and `*` (italic), methods with parameters and return types, `*bold*` / `_italic_` markdown |
| Relations | `<\|--`, `*--`, `o--`, `-->`, `..>`, `..\|>`, `--`, `..`, lollipops `()--`, cardinalities `"1" --> "*"`, labels |
| Grouping | `namespace` (nested, dotted names, `hierarchicalNamespaces`), `note`, `note for` |
| Styling | `style`, `classDef`, `cssClass`, `:::class`, `hideEmptyMembersBox` |
| Other | `<<annotation>>`, `direction`, `click` / `link` / `callback` with tooltips, `title`, `accTitle` / `accDescr`, `%%{init}%%` and front matter config |

The renderer follows Mermaid's `classBox` shape, `textHelper` metrics and
neo stylesheet. Mermaid draws outlines with rough.js, whose random control
points oxmaid replaces with fixed ones. Known differences: `htmlLabels: false`
still produces HTML labels, an empty namespace is not drawn, a namespace whose
title is wider than its content can overlap its neighbour, and nested
emphasis in one member keeps only the outer level. See
`tests/class_conformance.rs`, `tests/class_render.rs` and
`tools/conformance/class/`.

## Looks and themes

Mermaid 12 renders flowcharts with the `neo` look, the `redux-color` theme and
the ELK layout by default, and so does oxmaid: same stylesheet, markers, shape
geometry, Arial text metrics and spacing. Sequence diagrams follow the same
look. The previous look is available with `look: classic` in the front
matter, for both:

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
  class/
    mod.rs         class model (classes, members, relations, namespaces)
    parser.rs      port of Mermaid's Jison lexer and `classDb`
    tables.rs      Mermaid's compiled LALR parse tables
    render.rs      `classBox`-based renderer (neo look)
    neo.css        Mermaid's stylesheet for class diagrams
  sequence/
    mod.rs         sequence model (participants, messages, boxes)
    parser.rs      port of Mermaid's Jison lexer and `sequenceDb`
    render.rs      port of `sequenceRenderer` and `svgDraw` (both looks)
    neo.css        Mermaid's stylesheet for sequence diagrams
  config.rs        front matter and `%%{init}%%` configuration
  dom.rs           minimal element tree serialized like the browser's
  elk.rs           ELK-style geometry shared by flowcharts and class diagrams
  jison.rs         LALR driver for Mermaid's compiled Jison tables
  purify.rs        the parts of DOMPurify that Mermaid's sanitizer relies on
  layout.rs        layered (Sugiyama) layout, a stand-in for ELK and dagre
  text.rs          text measurement
  font_metrics.rs  Arial advance widths and kerning
  font_metrics_svg.rs  SVG text metrics (Times New Roman, Arial) from Chromium
  svg.rs           escaping and number formatting
```

Each diagram type gets its own module with a model, a parser and a renderer.
Layout and text measurement are shared.

- **Parser.** Mermaid uses Jison grammars. Flowcharts use a hand-written
  parser that follows the same syntax and reports precise positions; sequence
  and class diagrams port the Jison lexer rule by rule, and class diagrams
  run Mermaid's own compiled parse tables.
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
2. Sequence diagram themes other than `redux-color`, and `themeCSS`.
3. Class diagrams: SVG text labels (`htmlLabels: false`), the classic and
   hand-drawn looks.
4. Next diagram types: state, ER, gantt and pie charts.
5. Performance work: benchmarks against Mermaid, byte-level parsing,
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
reimplementation. The neo stylesheets (`src/flowchart/neo.css`,
`src/sequence/neo.css`, `src/class/neo.css`), arrow markers
(`src/flowchart/neo_markers.svg`, `src/class/neo_markers.svg`), the class
diagram parse tables (`src/class/tables.rs`) and
the sequence icons (`src/sequence/database_icon.txt`) are adapted from
Mermaid's output and sources,
© Knut Sveidqvist and contributors, under the MIT license.
