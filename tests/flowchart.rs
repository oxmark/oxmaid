use oxmaid::flowchart::{Arrow, Direction, EdgeEnd, NodeShape, Stroke};
use oxmaid::{Diagram, ErrorKind};

fn chart(src: &str) -> oxmaid::flowchart::Flowchart {
    match oxmaid::parse(src).unwrap() {
        Diagram::Flowchart(f) => f,
        #[allow(unreachable_patterns)]
        _ => panic!("not a flowchart"),
    }
}

#[test]
fn header_and_direction() {
    assert_eq!(chart("graph LR\nA").direction, Direction::LeftRight);
    assert_eq!(chart("flowchart").direction, Direction::TopBottom);
    assert_eq!(chart("graph TD;A-->B").edges.len(), 1);
}

#[test]
fn shapes() {
    let src = "flowchart TD\n a[r] & b(r) & c([s]) & d[[s]] & e[(c)] & f((c)) & g(((d))) & h{d} & i{{h}} & j>a] & k[/p/] & l[\\p\\] & m[/t\\] & n[\\t/] & o[\"quoted [x]\"] & p[/path]";
    let f = chart(src);
    let shapes: Vec<NodeShape> = f.nodes.iter().map(|n| n.shape).collect();
    use NodeShape::*;
    assert_eq!(
        shapes,
        [
            Rect,
            Round,
            Stadium,
            Subroutine,
            Cylinder,
            Circle,
            DoubleCircle,
            Diamond,
            Hexagon,
            Asymmetric,
            Parallelogram,
            ParallelogramAlt,
            Trapezoid,
            TrapezoidAlt,
            Rect,
            Rect
        ]
    );
    assert_eq!(f.node("o").unwrap().label, "quoted [x]");
    assert_eq!(f.node("p").unwrap().label, "/path");
}

#[test]
fn links() {
    let f = chart("graph TD\nA---B\nB-.->C\nC==>D\nD-- t1 -->E\nE-. t2 .->F\nF== t3 ==>G\nG<-->H\nH--oI\nI--xJ\nJ--->A\nK~~~L\nM-->|t4|N");
    let e: Vec<_> = f
        .edges
        .iter()
        .map(|e| (e.stroke, e.start, e.end, e.length, e.label.as_deref()))
        .collect();
    use Arrow::*;
    use Stroke::*;
    assert_eq!(
        e,
        [
            (Normal, None, None, 1, Option::None),
            (Dotted, None, Point, 1, Option::None),
            (Thick, None, Point, 1, Option::None),
            (Normal, None, Point, 1, Some("t1")),
            (Dotted, None, Point, 1, Some("t2")),
            (Thick, None, Point, 1, Some("t3")),
            (Normal, Point, Point, 1, Option::None),
            (Normal, None, Circle, 1, Option::None),
            (Normal, None, Cross, 1, Option::None),
            (Normal, None, Point, 2, Option::None),
            (Invisible, None, None, 1, Option::None),
            (Normal, None, Point, 1, Some("t4")),
        ]
    );
}

#[test]
fn chains_and_groups() {
    let f = chart("graph TD\nA & B --> C --> D & E");
    let pairs: Vec<(String, String)> = f
        .edges
        .iter()
        .map(|e| (f.end_id(e.from).to_string(), f.end_id(e.to).to_string()))
        .collect();
    let expected = [("A", "C"), ("B", "C"), ("C", "D"), ("C", "E")];
    assert_eq!(pairs, expected.map(|(a, b)| (a.to_string(), b.to_string())));
}

#[test]
fn ids_with_dashes_and_redefinition() {
    let f = chart("graph TD\nmy-node-->other.node\nmy-node[Label<br>two lines]");
    assert_eq!(f.nodes.len(), 2);
    assert_eq!(f.node("my-node").unwrap().label, "Label\ntwo lines");
    assert!(f.node("other.node").is_some());
}

#[test]
fn subgraphs_and_classes() {
    let f = chart(
        "---\ntitle: x\n---\n%% comment\nflowchart TB\nsubgraph one [First]\n a1 --> a2\n subgraph inner\n  a3\n end\nend\nsubgraph two\n b1\nend\nclassDef hot fill:#f96\nclass a1,b1 hot\na2:::hot\nstyle a3 fill:#bbf",
    );
    assert_eq!(f.subgraphs.len(), 3);
    assert_eq!(f.subgraphs[0].title, "First");
    assert_eq!(f.subgraphs[1].parent, Some(0));
    assert_eq!(f.node("a1").unwrap().subgraph, Some(0));
    assert_eq!(f.node("a3").unwrap().subgraph, Some(1));
    assert_eq!(f.node("b1").unwrap().subgraph, Some(2));
    assert_eq!(f.node("a1").unwrap().classes, ["hot"]);
    assert_eq!(f.node("a2").unwrap().classes, ["hot"]);
    assert_eq!(
        f.node("a3").unwrap().styles,
        [("fill".to_string(), "#bbf".to_string())]
    );
}

#[test]
fn links_to_subgraphs() {
    let f = chart(
        "flowchart TB\nsubgraph one\n a1\nend\nsubgraph two\n b1\nend\none --> two\nb1 --> one",
    );
    assert_eq!(f.nodes.len(), 2);
    assert_eq!(f.edges[0].from, EdgeEnd::Subgraph(0));
    assert_eq!(f.edges[0].to, EdgeEnd::Subgraph(1));
    assert_eq!(f.edges[1].from, EdgeEnd::Node(f.node_index("b1").unwrap()));
    assert!(oxmaid::render("flowchart TB\nsubgraph one\n a1\nend\nsubgraph two\n b1\nend\none --> two\nsubgraph empty\nend\nempty --> a1").is_ok());
}

#[test]
fn errors_have_positions() {
    let e = oxmaid::parse("flowchart TD\n  A --> \n").unwrap_err();
    assert_eq!((e.kind(), e.line()), (ErrorKind::Syntax, 2));
    let e = oxmaid::parse("flowchart TD\n  A[open --> B").unwrap_err();
    assert_eq!((e.line(), e.column()), (2, 4));
    let e = oxmaid::parse("flowchart TD\nsubgraph s\nA").unwrap_err();
    assert!(e.message().contains("never closed"));
    let e = oxmaid::parse("flowchart XY").unwrap_err();
    assert_eq!(e.column(), 11);
    assert_eq!(oxmaid::parse("").unwrap_err().kind(), ErrorKind::Empty);
    assert_eq!(
        oxmaid::parse("sequenceDiagram\nA->>B: hi")
            .unwrap_err()
            .kind(),
        ErrorKind::UnsupportedDiagram
    );
    assert_eq!(
        oxmaid::parse("nope").unwrap_err().kind(),
        ErrorKind::UnknownDiagram
    );
}

#[test]
fn renders_svg() {
    for dir in ["TD", "BT", "LR", "RL"] {
        let svg = oxmaid::render(&format!(
            "flowchart {dir}\nA[Start] -->|go| B{{Ok?}}\nB --> A\nB --> B\nC"
        ))
        .unwrap();
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains(">Start</tspan>") && svg.contains(">go</tspan>"));
        assert_eq!(svg.matches(r#"class="node""#).count(), 3);
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
}

#[test]
fn escapes_text_and_drops_unsafe_styles() {
    let svg =
        oxmaid::render("graph TD\nA[\"<script>&\"]\nstyle A fill:url(javascript:x),stroke:#f00")
            .unwrap();
    assert!(svg.contains("&lt;script&gt;&amp;"));
    assert!(!svg.contains("javascript"));
    assert!(svg.contains(r##"stroke="#f00""##));
}

#[test]
fn sample_diagrams_render() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/diagrams");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let src = std::fs::read_to_string(&path).unwrap();
        oxmaid::render(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}
