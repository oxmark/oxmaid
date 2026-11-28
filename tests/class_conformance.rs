//! Parses every class diagram of the Mermaid 12.3 corpus and compares the
//! model with what Mermaid's `classDb` holds.
//!
//! `tests/fixtures/class-mermaid-12.3.txt` is written by
//! `tools/conformance/class/fixture.py`: each case is `>>> source` (escaped)
//! followed by `error`, or by the dump lines that `dump` below reproduces.

use oxmaid::class::{ClassDiagram, MemberKind, RelationKind};
use oxmaid::Diagram;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map(esc).unwrap_or_default()
}

fn lst(v: &[String]) -> String {
    esc(&v.join("\x1f"))
}

fn kind(k: Option<RelationKind>) -> String {
    k.map_or("none".into(), |k| k.index().to_string())
}

fn dump(d: &ClassDiagram) -> Vec<String> {
    let mut out = vec![format!("dir\t{}", d.direction)];
    for c in &d.classes {
        let dom = c
            .dom_id
            .rsplit_once('-')
            .map_or(c.dom_id.as_str(), |(a, _)| a);
        out.push(
            [
                "class".to_string(),
                esc(&c.id),
                esc(&c.id),
                esc(&c.generic),
                esc(&c.label),
                esc(&c.text),
                esc(&c.css_classes),
                lst(&c.annotations),
                lst(&c.styles),
                esc(dom),
                opt(&c.parent),
                opt(&c.link),
                opt(&c.link_target),
                if c.have_callback { "1" } else { "" }.to_string(),
                opt(&c.tooltip),
            ]
            .join("\t"),
        );
        for (k, ms) in [("attr", &c.members), ("meth", &c.methods)] {
            for m in ms {
                let method = m.kind == MemberKind::Method;
                out.push(
                    [
                        k.to_string(),
                        esc(&m.visibility),
                        esc(&m.classifier),
                        esc(&m.id),
                        if method {
                            esc(&m.parameters)
                        } else {
                            String::new()
                        },
                        if method {
                            esc(&m.return_type)
                        } else {
                            String::new()
                        },
                        esc(&m.text),
                    ]
                    .join("\t"),
                );
            }
        }
    }
    for r in &d.relations {
        out.push(
            [
                "rel".to_string(),
                esc(&r.id1),
                esc(&r.id2),
                kind(r.type1),
                kind(r.type2),
                u8::from(r.dotted).to_string(),
                esc(&r.title1),
                esc(&r.title2),
                opt(&r.title),
            ]
            .join("\t"),
        );
    }
    for n in &d.notes {
        out.push(
            [
                "note".to_string(),
                esc(&n.id),
                opt(&n.class),
                esc(&n.text),
                opt(&n.parent),
            ]
            .join("\t"),
        );
    }
    for n in &d.namespaces {
        out.push(
            [
                "ns".to_string(),
                esc(&n.id),
                esc(&n.label),
                opt(&n.parent),
                if n.explicit { "1" } else { "" }.to_string(),
                lst(&n.classes),
                lst(&n.notes),
                lst(&n.children),
            ]
            .join("\t"),
        );
    }
    for s in &d.style_classes {
        out.push(
            [
                "def".to_string(),
                esc(&s.id),
                lst(&s.styles),
                lst(&s.text_styles),
            ]
            .join("\t"),
        );
    }
    for i in &d.interfaces {
        out.push(format!(
            "iface\t{}\t{}\t{}",
            esc(&i.id),
            esc(&i.label),
            esc(&i.class_id)
        ));
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

#[test]
fn class_parser_matches_mermaid() {
    let fixture = include_str!("fixtures/class-mermaid-12.3.txt");
    let mut cases: Vec<(String, Vec<&str>)> = Vec::new();
    for line in fixture.lines() {
        match line.strip_prefix(">>> ") {
            Some(src) => cases.push((unesc(src), Vec::new())),
            None => cases.last_mut().expect("case header").1.push(line),
        }
    }
    let mut failures = Vec::new();
    for (i, (src, want)) in cases.iter().enumerate() {
        let got = match oxmaid::parse(src) {
            Ok(Diagram::Class(d)) => dump(&d),
            Ok(_) => vec!["not a class diagram".into()],
            Err(_) => vec!["error".into()],
        };
        if got != *want {
            let diff: Vec<String> = want
                .iter()
                .filter(|l| !got.iter().any(|g| g == *l))
                .map(|l| format!("- {l}"))
                .chain(
                    got.iter()
                        .filter(|g| !want.contains(&g.as_str()))
                        .map(|g| format!("+ {g}")),
                )
                .take(10)
                .collect();
            failures.push(format!(
                "case {i}:\n{}\n{}",
                src.trim_end(),
                diff.join("\n")
            ));
        }
    }
    assert!(cases.len() > 400, "fixture holds {} cases", cases.len());
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Mermaid 12.3:\n\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
}
