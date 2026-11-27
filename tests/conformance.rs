//! Differential test against Mermaid 12.3.
//!
//! `tests/fixtures/flowchart-mermaid-12.3.txt` holds what Mermaid's own
//! `flowDb` returns for the inputs of its flowchart spec files plus generated
//! link, length and keyword combinations. Each case starts with `>>> source`
//! (escaped) and lists one tab-separated line per direction, node, edge,
//! subgraph, accessibility field and class definition, or `error`.
//!
//! Set `OXMAID_CONFORMANCE_OUT=path` to write oxmaid's own dump for diffing.

use oxmaid::flowchart::{Arrow, Direction, Flowchart, LabelType, NodeShape, Stroke};
use oxmaid::Diagram;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn dir(d: Direction) -> &'static str {
    match d {
        Direction::TopBottom => "TB",
        Direction::BottomTop => "BT",
        Direction::LeftRight => "LR",
        Direction::RightLeft => "RL",
    }
}

fn label_type(t: LabelType) -> &'static str {
    match t {
        LabelType::Text => "text",
        LabelType::String => "string",
        LabelType::Markdown => "markdown",
    }
}

fn css(props: &[(String, String)]) -> String {
    props
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                k.clone()
            } else {
                format!("{k}:{v}")
            }
        })
        .map(|p| p.split_whitespace().collect::<String>())
        .collect::<Vec<_>>()
        .join(";")
}

fn arrow(a: Arrow) -> &'static str {
    match a {
        Arrow::None => "open",
        Arrow::Point => "point",
        Arrow::Circle => "circle",
        Arrow::Cross => "cross",
    }
}

fn dump(f: &Flowchart) -> Vec<String> {
    let mut out = vec![format!("dir\t{}", dir(f.direction))];
    for n in &f.nodes {
        out.push(
            [
                "node",
                &esc(&n.id),
                &esc(&n.label),
                label_type(n.label_type),
                n.shape.name(),
                &n.classes.join(","),
                &esc(&css(&n.styles)),
                &esc(n.link.as_deref().unwrap_or("")),
                &esc(n.link_target.as_deref().unwrap_or("")),
                &format!("flowchart-{}-{}", esc(&n.id), n.dom_index),
            ]
            .join("\t"),
        );
    }
    for e in &f.edges {
        let ty = if e.start != Arrow::None {
            format!("double_arrow_{}", arrow(e.end))
        } else {
            format!("arrow_{}", arrow(e.end))
        };
        let stroke = match e.stroke {
            Stroke::Normal => "normal",
            Stroke::Dotted => "dotted",
            Stroke::Thick => "thick",
            Stroke::Invisible => "invisible",
        };
        out.push(
            [
                "edge",
                &esc(&e.id),
                &esc(f.end_id(e.from)),
                &esc(f.end_id(e.to)),
                &ty,
                stroke,
                &e.length.to_string(),
                &esc(e.label.as_deref().unwrap_or("")),
                label_type(e.label_type),
                &esc(&css(&e.styles)),
                &esc(e.curve.as_deref().unwrap_or("")),
                if e.animate { "animate" } else { "" },
            ]
            .join("\t"),
        );
    }
    for (i, s) in f.subgraphs.iter().enumerate() {
        let mut members: Vec<&str> = f
            .nodes
            .iter()
            .filter(|n| n.subgraph == Some(i))
            .map(|n| n.id.as_str())
            .chain(
                f.subgraphs
                    .iter()
                    .filter(|c| c.parent == Some(i))
                    .map(|c| c.id.as_str()),
            )
            .collect();
        members.sort_unstable();
        members.dedup();
        out.push(
            [
                "sub",
                &esc(&s.id),
                &esc(&s.title),
                &members.join(","),
                s.direction.map(dir).unwrap_or(""),
                &s.classes.join(","),
            ]
            .join("\t"),
        );
    }
    if f.acc_title.is_some() || f.acc_descr.is_some() {
        out.push(format!(
            "acc\t{}\t{}",
            esc(f.acc_title.as_deref().unwrap_or("")),
            esc(f.acc_descr.as_deref().unwrap_or(""))
        ));
    }
    for c in &f.class_defs {
        out.push(format!(
            "classdef\t{}\t{}",
            esc(&c.name),
            esc(&css(&c.styles))
        ));
    }
    out
}

/// Mermaid stores shapes by their `type`; map them to our canonical names.
fn normalize(line: &str) -> String {
    let mut fields: Vec<String> = line.split('\t').map(str::to_string).collect();
    if fields[0] == "node" {
        if let Some(shape) = NodeShape::from_name(&fields[4]) {
            fields[4] = shape.name().to_string();
        }
    }
    fields.join("\t")
}

#[test]
fn matches_mermaid_flow_db() {
    let fixture = include_str!("fixtures/flowchart-mermaid-12.3.txt");
    let mut cases: Vec<(String, Vec<String>)> = Vec::new();
    for line in fixture.lines() {
        if let Some(src) = line.strip_prefix(">>> ") {
            cases.push((unesc(src), Vec::new()));
        } else if let Some(case) = cases.last_mut() {
            case.1.push(normalize(line));
        }
    }

    let mut actual_dump = String::new();
    let mut expected_dump = String::new();
    let mut failures = Vec::new();
    for (src, expected) in &cases {
        let actual = match oxmaid::parse(src) {
            Ok(Diagram::Flowchart(f)) => dump(&f),
            #[allow(unreachable_patterns)]
            Ok(_) => vec!["other".to_string()],
            Err(_) => vec!["error".to_string()],
        };
        expected_dump.push_str(&format!(">>> {}\n{}\n", esc(src), expected.join("\n")));
        actual_dump.push_str(&format!(">>> {}\n{}\n", esc(src), actual.join("\n")));
        if &actual != expected {
            failures.push(format!(
                "--- {src:?}\nexpected:\n  {}\nactual:\n  {}",
                expected.join("\n  "),
                actual.join("\n  ")
            ));
        }
    }
    if let Ok(path) = std::env::var("OXMAID_CONFORMANCE_OUT") {
        std::fs::write(format!("{path}.expected"), expected_dump).unwrap();
        std::fs::write(path, actual_dump).unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Mermaid 12.3:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
