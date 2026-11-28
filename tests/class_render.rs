//! Renders every class diagram of the Mermaid 12.3 corpus that uses the
//! default setup (ELK layout, neo look, default theme, HTML labels) and checks
//! each node's box against what Mermaid draws: its size and where the
//! annotation, name, members and methods sit inside it.
//!
//! `tests/fixtures/class-boxes-mermaid-12.3.txt` is written by
//! `tools/conformance/class/boxes.py`.

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

/// A node box: id, half width, half height and `(group, x, y)` offsets.
type Box = (String, f64, f64, Vec<(String, f64, f64)>);

fn num(s: &str) -> f64 {
    s.trim().parse().unwrap_or(f64::NAN)
}

fn after<'a>(s: &'a str, pat: &str) -> Option<&'a str> {
    s.find(pat).map(|i| &s[i + pat.len()..])
}

fn until(s: &str, end: char) -> &str {
    s.split(end).next().unwrap_or("")
}

/// Reads the boxes out of an oxmaid SVG, as `boxes.py` does for Mermaid's.
fn boxes(svg: &str) -> Vec<Box> {
    let mut out = Vec::new();
    for part in svg.split(r#"<g class="node "#).skip(1) {
        let Some(id) = after(part, r#"id="oxmaid-"#).map(|s| until(s, '"')) else {
            continue;
        };
        let (w, h) = if let Some(p) = after(part, r#"<path d="M"#) {
            let mut it = p.split(' ');
            (num(it.next().unwrap()), num(it.next().unwrap()))
        } else {
            let r = after(part, "<rect").unwrap();
            (
                num(until(after(r, r#"x=""#).unwrap(), '"')),
                num(until(after(r, r#"y=""#).unwrap(), '"')),
            )
        };
        let mut groups = Vec::new();
        let mut rest = part;
        while let Some(i) = rest.find(r#"-group text" transform="translate("#) {
            let name = rest[..i].rsplit('"').next().unwrap().to_string();
            let t = until(&rest[i..], ')');
            let t = &t[t.find('(').unwrap() + 1..];
            let (x, y) = t.split_once(',').unwrap();
            groups.push((name, num(x), num(y)));
            rest = &rest[i + 1..];
        }
        out.push((id.to_string(), w.abs(), h.abs(), groups));
    }
    out
}

fn parse_expected(lines: &[&str]) -> Vec<Box> {
    lines
        .iter()
        .map(|l| {
            let mut f = l.split('\t');
            let id = unesc(f.next().unwrap());
            let w = num(f.next().unwrap());
            let h = num(f.next().unwrap());
            let groups = f
                .map(|g| {
                    let (name, xy) = g.split_once('=').unwrap();
                    let (x, y) = xy.split_once(',').unwrap();
                    (name.to_string(), num(x), num(y))
                })
                .collect();
            (id, w, h, groups)
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    // The fixture keeps six significant digits.
    (a - b).abs() <= 1e-3 * a.abs().max(1.0)
}

fn same(a: &Box, b: &Box) -> bool {
    a.0 == b.0
        && close(a.1, b.1)
        && close(a.2, b.2)
        && a.3.len() == b.3.len()
        && a.3
            .iter()
            .zip(&b.3)
            .all(|(x, y)| x.0 == y.0 && close(x.1, y.1) && close(x.2, y.2))
}

#[test]
fn class_boxes_match_mermaid() {
    let fixture = include_str!("fixtures/class-boxes-mermaid-12.3.txt");
    let mut cases: Vec<(String, Vec<&str>)> = Vec::new();
    for line in fixture.lines() {
        match line.strip_prefix(">>> ") {
            Some(src) => cases.push((unesc(src), Vec::new())),
            None => cases.last_mut().expect("case header").1.push(line),
        }
    }
    let mut failures = Vec::new();
    for (i, (src, lines)) in cases.iter().enumerate() {
        let want = parse_expected(lines);
        let got = match oxmaid::render(src) {
            Ok(svg) => boxes(&svg),
            Err(e) => {
                failures.push(format!("case {i}: {e}"));
                continue;
            }
        };
        let ok = want.len() == got.len() && want.iter().zip(&got).all(|(a, b)| same(a, b));
        if !ok {
            failures.push(format!(
                "case {i}:\n{}\nwant {want:?}\n got {got:?}",
                src.trim_end()
            ));
        }
    }
    assert!(cases.len() > 200, "fixture holds {} cases", cases.len());
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Mermaid 12.3:\n\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
}

#[test]
fn class_svg_has_expected_parts() {
    let svg = oxmaid::render(
        "classDiagram\n\
         note \"A note\"\n\
         Animal <|-- Duck : extends\n\
         Animal \"1\" *-- \"many\" Leg\n\
         class Animal{\n<<abstract>>\n+int age\n+isMammal() bool\n}\n\
         namespace Pond {\nclass Duck\n}\n\
         click Duck href \"https://example.com\" \"Quack\"\n",
    )
    .unwrap();
    for part in [
        r#"aria-roledescription="classDiagram""#,
        r#"<g class="cluster undefined" id="oxmaid-Pond""#,
        r#"id="oxmaid-classId-Animal-0""#,
        "«abstract»",
        "+isMammal() : bool",
        r#"marker-start="url(#oxmaid_classDiagram-extensionStart-margin)""#,
        r#"marker-start="url(#oxmaid_classDiagram-compositionStart-margin)""#,
        r#"<g class="edgeTerminals""#,
        "<p>many</p>",
        r#"<a xlink:href="https://example.com/""#,
        r#"id="oxmaid-note0""#,
    ] {
        assert!(svg.contains(part), "missing {part} in\n{svg}");
    }
}
