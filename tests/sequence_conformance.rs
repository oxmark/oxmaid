//! Differential test of the sequence parser against Mermaid 12.3.
//!
//! `tests/fixtures/sequence-mermaid-12.3.txt` holds what Mermaid's own
//! `sequenceDb` returns for its sequence e2e corpus and the diagrams of its
//! sequence spec files. Each case starts with `>>> source` (escaped) and lists
//! one tab-separated line per participant, message, box, created or destroyed
//! participant and title, or `error`.
//!
//! Set `OXMAID_CONFORMANCE_OUT=path` to write oxmaid's own dump for diffing.

use oxmaid::sequence::{Placement, SequenceDiagram};
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

fn flag(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        ""
    }
}

fn num(v: Option<f64>) -> String {
    v.map(|v| format!("{v}")).unwrap_or_default()
}

fn opt(s: &Option<String>) -> String {
    esc(s.as_deref().unwrap_or(""))
}

fn dump(d: &SequenceDiagram) -> Vec<String> {
    let mut out = Vec::new();
    for a in &d.participants {
        let links: Vec<String> = a.links.iter().map(|(k, v)| format!("{k}={v}")).collect();
        out.push(
            [
                "actor".to_string(),
                esc(&a.id),
                esc(&a.name),
                esc(&a.description),
                flag(a.wrap).into(),
                esc(a.kind.name()),
                a.group.map(|g| g.to_string()).unwrap_or_default(),
                opt(&a.prev),
                opt(&a.next),
                esc(&links.join(";")),
                opt(&a.class),
                opt(&a.icon),
            ]
            .join("\t"),
        );
    }
    for m in &d.messages {
        if let Some(a) = &m.autonumber {
            out.push(format!(
                "auto\t{}\t{}\t{}",
                num(a.start),
                num(a.step),
                flag(a.visible)
            ));
            continue;
        }
        let placement = match m.placement {
            Some(Placement::LeftOf) => "left",
            Some(Placement::RightOf) => "right",
            Some(Placement::Over) => "over",
            None => "",
        };
        out.push(
            [
                "msg".to_string(),
                m.kind.code().to_string(),
                opt(&m.from),
                opt(&m.to),
                esc(&m.text),
                flag(m.wrap).into(),
                flag(m.activate).into(),
                placement.into(),
                m.central.map(|c| c.code().to_string()).unwrap_or_default(),
            ]
            .join("\t"),
        );
    }
    for b in &d.boxes {
        out.push(format!(
            "box\t{}\t{}\t{}\t{}",
            opt(&b.name),
            esc(&b.fill),
            flag(b.wrap),
            esc(&b.participants.join(","))
        ));
    }
    for (k, v) in &d.created {
        out.push(format!("created\t{}\t{v}", esc(k)));
    }
    for (k, v) in &d.destroyed {
        out.push(format!("destroyed\t{}\t{v}", esc(k)));
    }
    if d.title.is_some() || d.acc_title.is_some() || d.acc_descr.is_some() {
        out.push(format!(
            "meta\t{}\t{}\t{}",
            opt(&d.title),
            opt(&d.acc_title),
            opt(&d.acc_descr)
        ));
    }
    out
}

#[test]
fn matches_mermaid_sequence_db() {
    let fixture = include_str!("fixtures/sequence-mermaid-12.3.txt");
    let mut cases: Vec<(String, Vec<String>)> = Vec::new();
    for line in fixture.lines() {
        if let Some(src) = line.strip_prefix(">>> ") {
            cases.push((unesc(src), Vec::new()));
        } else if let Some(case) = cases.last_mut() {
            case.1.push(line.to_string());
        }
    }

    let mut actual_dump = String::new();
    let mut expected_dump = String::new();
    let mut failures = Vec::new();
    for (src, expected) in &cases {
        let actual = match oxmaid::parse(src) {
            Ok(Diagram::Sequence(d)) => dump(&d),
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
