//! Byte-for-byte test of the sequence renderer against Mermaid 12.3.
//!
//! `tests/fixtures/sequence-svg-mermaid-12.3.txt` lists every diagram of the
//! sequence corpus that Mermaid draws with its default theme (`redux-color`,
//! neo and classic looks), as `>>> source` (escaped) followed by the FNV-1a 64
//! hash and the length of the SVG Mermaid rendered with the id `d`. It is
//! written by `tools/conformance/sequence/svg_fixture.py`.
//!
//! Set `OXMAID_CONFORMANCE_OUT=dir` to write oxmaid's SVG of each case to
//! `dir/<n>.svg` for diffing against the oracle's output.

use oxmaid::RenderOptions;

/// Cases that still differ from Mermaid, by the hash of Mermaid's SVG.
const KNOWN: &[(&str, &str)] = &[
    // `themeCSS` is not supported.
    (
        "34be7e615feb0a09",
        "should-render-alt-else-section-titles-with-label-box-backgrounds",
    ),
    // Classic-look database labels sit on Chromium's float32 bounding box of
    // the cylinder path (e.g. `y="229.00000381469727"`).
    ("14ce16b3f19755c0", "v2-classic-boxes-around-groups"),
    (
        "cfdc5f9fd264b5a5",
        "v2-classic-participant-creation-and-destruction",
    ),
    (
        "5362cf650e6d9cb0",
        "should-render-stereotypes-with-aliases-in-boxes",
    ),
];

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

fn fnv1a64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| {
        (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn sequence_svg_matches_mermaid() {
    let fixture = include_str!("fixtures/sequence-svg-mermaid-12.3.txt");
    let out_dir = std::env::var_os("OXMAID_CONFORMANCE_OUT").map(std::path::PathBuf::from);
    let options = RenderOptions {
        id_prefix: "d".into(),
        ..RenderOptions::default()
    };
    let mut lines = fixture.lines();
    let (mut total, mut failures) = (0, Vec::new());
    while let Some(head) = lines.next() {
        let src = unesc(head.strip_prefix(">>> ").expect("case header"));
        let (hash, len) = lines.next().expect("hash line").split_once('\t').unwrap();
        let svg = oxmaid::parse(&src)
            .unwrap_or_else(|e| panic!("case {total} does not parse: {e}\n{src}"))
            .to_svg(&options);
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(format!("{total}.svg")), &svg).unwrap();
        }
        let got = format!("{:016x}", fnv1a64(svg.as_bytes()));
        let known = KNOWN.iter().any(|(h, _)| *h == hash);
        if (got != hash || svg.len().to_string() != len) && !known {
            failures.push(format!(
                "case {total}: {} bytes, expected {len}\n{}",
                svg.len(),
                src.lines().take(12).collect::<Vec<_>>().join("\n")
            ));
        }
        total += 1;
    }
    assert!(total > 150, "fixture holds {total} cases");
    assert!(
        failures.is_empty(),
        "{} of {total} SVGs differ from Mermaid 12.3:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
