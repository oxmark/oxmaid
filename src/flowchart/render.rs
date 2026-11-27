//! Flowchart to SVG.

use std::fmt::Write;

use super::{Arrow, Direction, EdgeEnd, Flowchart, Node, NodeShape, Stroke};
use crate::layout::{self, LayoutCluster, LayoutConfig, LayoutEdge, LayoutNode, Point};
use crate::svg::{esc, num, safe_css_value};
use crate::text::{self, TextBox};
use crate::RenderOptions;

/// Padding between a label and its shape (Mermaid's flowchart `padding`).
const NODE_PADDING: f64 = 15.0;
const EDGE_LABEL_PADDING: f64 = 4.0;
const CLUSTER_PADDING: f64 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    fn around(c: Point, w: f64, h: f64) -> Self {
        Self {
            x0: c.x - w / 2.0,
            y0: c.y - h / 2.0,
            x1: c.x + w / 2.0,
            y1: c.y + h / 2.0,
        }
    }

    fn union(self, o: Rect) -> Rect {
        Rect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    fn contains(&self, p: Point) -> bool {
        p.x > self.x0 && p.x < self.x1 && p.y > self.y0 && p.y < self.y1
    }
}

/// Cuts the start of a route at the border of `r`: drops the points inside
/// it and ends the route where it leaves the box.
fn trim_start(pts: &mut Vec<Point>, r: &Rect) {
    let Some(k) = pts.iter().position(|p| !r.contains(*p)) else {
        return;
    };
    if k == 0 {
        return;
    }
    let (mut a, mut b) = (pts[k - 1], pts[k]);
    for _ in 0..40 {
        let m = Point {
            x: (a.x + b.x) / 2.0,
            y: (a.y + b.y) / 2.0,
        };
        if r.contains(m) {
            a = m;
        } else {
            b = m;
        }
    }
    pts.splice(0..k, [b]);
}

/// Nodes inside subgraph `s` or any of its descendants, in definition order.
fn members(fc: &Flowchart, s: usize) -> Vec<usize> {
    let inside = |mut sg: Option<usize>| {
        while let Some(k) = sg {
            if k == s {
                return true;
            }
            sg = fc.subgraphs[k].parent;
        }
        false
    };
    (0..fc.nodes.len())
        .filter(|&i| inside(fc.nodes[i].subgraph))
        .collect()
}

struct NodeBox {
    w: f64,
    h: f64,
    text: TextBox,
}

fn node_box(node: &Node, font_size: f64, line_height: f64) -> NodeBox {
    let text = text::measure(&node.label, font_size, line_height);
    let tw = text.width.max(font_size * 0.5);
    let th = text.height.max(line_height);
    let (w, h) = (tw + 2.0 * NODE_PADDING, th + 2.0 * NODE_PADDING);
    let (w, h) = match node.shape {
        NodeShape::Rect | NodeShape::Round => (w, h),
        NodeShape::Stadium => (w + h / 2.0, h),
        NodeShape::Subroutine => (w + 16.0, h),
        NodeShape::Cylinder => (w, h + cylinder_ry(w)),
        NodeShape::Circle => {
            let d = w.max(h);
            (d, d)
        }
        NodeShape::DoubleCircle => {
            let d = w.max(h) + 10.0;
            (d, d)
        }
        NodeShape::Diamond => {
            let s = w + h;
            (s, s)
        }
        NodeShape::Hexagon => (w + h / 2.0, h),
        NodeShape::Asymmetric => (w + h / 4.0, h),
        NodeShape::Parallelogram
        | NodeShape::ParallelogramAlt
        | NodeShape::Trapezoid
        | NodeShape::TrapezoidAlt => (w + h / 3.0, h),
    };
    NodeBox { w, h, text }
}

fn cylinder_ry(w: f64) -> f64 {
    (w / 2.0) / (2.5 + w / 50.0)
}

/// Point where the segment from the node center towards `toward` leaves the shape.
fn clip(shape: NodeShape, c: Point, w: f64, h: f64, toward: Point) -> Point {
    let (dx, dy) = (toward.x - c.x, toward.y - c.y);
    if dx == 0.0 && dy == 0.0 {
        return c;
    }
    let t = match shape {
        NodeShape::Circle | NodeShape::DoubleCircle => (w / 2.0) / dx.hypot(dy),
        NodeShape::Diamond => 1.0 / (dx.abs() / (w / 2.0) + dy.abs() / (h / 2.0)),
        _ => {
            let tx = if dx != 0.0 {
                (w / 2.0) / dx.abs()
            } else {
                f64::INFINITY
            };
            let ty = if dy != 0.0 {
                (h / 2.0) / dy.abs()
            } else {
                f64::INFINITY
            };
            tx.min(ty)
        }
    };
    let t = t.min(1.0);
    Point {
        x: c.x + dx * t,
        y: c.y + dy * t,
    }
}

/// SVG path through `pts` with d3's `curveBasis` (Mermaid's default curve).
fn basis_path(pts: &[Point]) -> String {
    let mut d = String::new();
    if pts.len() < 3 {
        for (i, p) in pts.iter().enumerate() {
            let _ = write!(
                d,
                "{}{},{}",
                if i == 0 { "M" } else { "L" },
                num(p.x),
                num(p.y)
            );
        }
        return d;
    }
    let bezier = |d: &mut String, p0: Point, p1: Point, p: Point| {
        let _ = write!(
            d,
            "C{},{},{},{},{},{}",
            num((2.0 * p0.x + p1.x) / 3.0),
            num((2.0 * p0.y + p1.y) / 3.0),
            num((p0.x + 2.0 * p1.x) / 3.0),
            num((p0.y + 2.0 * p1.y) / 3.0),
            num((p0.x + 4.0 * p1.x + p.x) / 6.0),
            num((p0.y + 4.0 * p1.y + p.y) / 6.0)
        );
    };
    let _ = write!(d, "M{},{}", num(pts[0].x), num(pts[0].y));
    let (p0, p1) = (pts[0], pts[1]);
    let _ = write!(
        d,
        "L{},{}",
        num((5.0 * p0.x + p1.x) / 6.0),
        num((5.0 * p0.y + p1.y) / 6.0)
    );
    for i in 2..pts.len() {
        bezier(&mut d, pts[i - 2], pts[i - 1], pts[i]);
    }
    let (a, b) = (pts[pts.len() - 2], pts[pts.len() - 1]);
    bezier(&mut d, a, b, b);
    let _ = write!(d, "L{},{}", num(b.x), num(b.y));
    d
}

struct Paint {
    fill: String,
    stroke: String,
    stroke_width: String,
    dash: Option<String>,
    color: String,
}

fn node_paint(fc: &Flowchart, node: &Node, o: &RenderOptions) -> Paint {
    let mut p = Paint {
        fill: o.theme.node_fill.clone(),
        stroke: o.theme.node_stroke.clone(),
        stroke_width: "1".into(),
        dash: None,
        color: o.theme.text.clone(),
    };
    let class_styles = std::iter::once("default")
        .chain(node.classes.iter().map(String::as_str))
        .filter_map(|name| fc.class_defs.iter().find(|c| c.name == name))
        .flat_map(|c| c.styles.iter());
    for (k, v) in class_styles.chain(node.styles.iter()) {
        let Some(v) = safe_css_value(v) else { continue };
        match k.as_str() {
            "fill" => p.fill = v.into(),
            "stroke" => p.stroke = v.into(),
            "stroke-width" => p.stroke_width = v.into(),
            "stroke-dasharray" => p.dash = Some(v.into()),
            "color" => p.color = v.into(),
            _ => {}
        }
    }
    p
}

fn text_block(out: &mut String, c: Point, lines: &[String], line_height: f64, color: &str) {
    if lines.is_empty() {
        return;
    }
    let y0 = c.y - (lines.len() - 1) as f64 * line_height / 2.0;
    let _ = write!(
        out,
        r#"<text text-anchor="middle" dominant-baseline="central" fill="{}">"#,
        esc(color)
    );
    for (i, line) in lines.iter().enumerate() {
        let _ = write!(
            out,
            r#"<tspan x="{}" y="{}">{}</tspan>"#,
            num(c.x),
            num(y0 + i as f64 * line_height),
            esc(line)
        );
    }
    out.push_str("</text>");
}

fn polygon(points: &[(f64, f64)]) -> String {
    let pts: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("{},{}", num(*x), num(*y)))
        .collect();
    format!(r#"<polygon points="{}""#, pts.join(" "))
}

fn shape_svg(shape: NodeShape, c: Point, w: f64, h: f64, paint_attrs: &str) -> String {
    let (x, y) = (c.x - w / 2.0, c.y - h / 2.0);
    let rect = |rx: f64| {
        format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}"{paint_attrs}/>"#,
            num(x),
            num(y),
            num(w),
            num(h),
            num(rx)
        )
    };
    let k = h / 3.0;
    match shape {
        NodeShape::Rect => rect(0.0),
        NodeShape::Round => rect(5.0),
        NodeShape::Stadium => rect(h / 2.0),
        NodeShape::Subroutine => format!(
            r#"{}<path d="M{},{}V{}M{},{}V{}" fill="none"{paint_attrs}/>"#,
            rect(0.0),
            num(x + 8.0),
            num(y),
            num(y + h),
            num(x + w - 8.0),
            num(y),
            num(y + h)
        ),
        NodeShape::Cylinder => {
            let rx = w / 2.0;
            let ry = cylinder_ry(w);
            let body = h - 2.0 * ry;
            format!(
                r#"<path d="M{x0},{y0}a{rx},{ry} 0 0 0 {w},0a{rx},{ry} 0 0 0 -{w},0l0,{body}a{rx},{ry} 0 0 0 {w},0l0,-{body}"{paint_attrs}/>"#,
                x0 = num(x),
                y0 = num(y + ry),
                rx = num(rx),
                ry = num(ry),
                w = num(w),
                body = num(body)
            )
        }
        NodeShape::Circle => {
            format!(
                r#"<circle cx="{}" cy="{}" r="{}"{paint_attrs}/>"#,
                num(c.x),
                num(c.y),
                num(w / 2.0)
            )
        }
        NodeShape::DoubleCircle => format!(
            r#"<circle cx="{cx}" cy="{cy}" r="{}"{paint_attrs}/><circle cx="{cx}" cy="{cy}" r="{}"{paint_attrs}/>"#,
            num(w / 2.0),
            num(w / 2.0 - 5.0),
            cx = num(c.x),
            cy = num(c.y)
        ),
        NodeShape::Diamond => {
            polygon(&[(c.x, y), (x + w, c.y), (c.x, y + h), (x, c.y)]) + paint_attrs + "/>"
        }
        NodeShape::Hexagon => {
            let m = h / 4.0;
            polygon(&[
                (x + m, y),
                (x + w - m, y),
                (x + w, c.y),
                (x + w - m, y + h),
                (x + m, y + h),
                (x, c.y),
            ]) + paint_attrs
                + "/>"
        }
        NodeShape::Asymmetric => {
            polygon(&[
                (x, y),
                (x + w, y),
                (x + w, y + h),
                (x, y + h),
                (x + h / 4.0, c.y),
            ]) + paint_attrs
                + "/>"
        }
        NodeShape::Parallelogram => {
            polygon(&[(x + k, y), (x + w, y), (x + w - k, y + h), (x, y + h)]) + paint_attrs + "/>"
        }
        NodeShape::ParallelogramAlt => {
            polygon(&[(x, y), (x + w - k, y), (x + w, y + h), (x + k, y + h)]) + paint_attrs + "/>"
        }
        NodeShape::Trapezoid => {
            polygon(&[(x + k, y), (x + w - k, y), (x + w, y + h), (x, y + h)]) + paint_attrs + "/>"
        }
        NodeShape::TrapezoidAlt => {
            polygon(&[(x, y), (x + w, y), (x + w - k, y + h), (x + k, y + h)]) + paint_attrs + "/>"
        }
    }
}

fn marker_ref(prefix: &str, arrow: Arrow) -> Option<String> {
    let name = match arrow {
        Arrow::None => return None,
        Arrow::Point => "point",
        Arrow::Circle => "circle",
        Arrow::Cross => "cross",
    };
    Some(format!("url(#{}-{name})", esc(prefix)))
}

pub(crate) fn render(fc: &Flowchart, o: &RenderOptions) -> String {
    let fs = o.font_size;
    let lh = fs * 1.5;
    let dir = fc.direction;
    let horizontal = dir.is_horizontal();
    let swap = |w: f64, h: f64| if horizontal { (h, w) } else { (w, h) };

    let boxes: Vec<NodeBox> = fc.nodes.iter().map(|n| node_box(n, fs, lh)).collect();
    let label_boxes: Vec<Option<TextBox>> = fc
        .edges
        .iter()
        .map(|e| e.label.as_ref().map(|l| text::measure(l, fs, lh)))
        .collect();
    let label_size = |t: &TextBox| {
        (
            t.width + 2.0 * EDGE_LABEL_PADDING,
            t.height + EDGE_LABEL_PADDING,
        )
    };

    let lnodes: Vec<LayoutNode> = boxes
        .iter()
        .zip(&fc.nodes)
        .map(|(b, n)| {
            let (width, height) = swap(b.w, b.h);
            LayoutNode {
                width,
                height,
                cluster: n.subgraph,
            }
        })
        .collect();
    let title_height = |s: usize| {
        if fc.subgraphs[s].title.is_empty() {
            0.0
        } else {
            lh
        }
    };
    let lclusters: Vec<LayoutCluster> = (0..fc.subgraphs.len())
        .map(|s| LayoutCluster {
            parent: fc.subgraphs[s].parent,
            padding: CLUSTER_PADDING + title_height(s),
        })
        .collect();
    // A link to a subgraph is laid out as a link to one of its nodes (the
    // last one for a source, the first one for a target), then cut at the
    // subgraph border once the boxes are known.
    let sg_members: Vec<Vec<usize>> = (0..fc.subgraphs.len()).map(|s| members(fc, s)).collect();
    let layout_end = |end: EdgeEnd, source: bool| match end {
        EdgeEnd::Node(i) => Some(i),
        EdgeEnd::Subgraph(s) => {
            if source {
                sg_members[s].last().copied()
            } else {
                sg_members[s].first().copied()
            }
        }
    };
    let ends: Vec<Option<(usize, usize)>> = fc
        .edges
        .iter()
        .map(|e| Some((layout_end(e.from, true)?, layout_end(e.to, false)?)))
        .collect();
    let ledges: Vec<LayoutEdge> = fc
        .edges
        .iter()
        .zip(&label_boxes)
        .zip(&ends)
        .map(|((e, lb), ends)| {
            // Unresolvable ends (empty subgraph) become ignored self loops.
            let (from, to) = ends.unwrap_or((0, 0));
            LayoutEdge {
                from,
                to,
                min_len: e.length,
                label: lb.as_ref().map(|t| {
                    let (w, h) = label_size(t);
                    swap(w, h)
                }),
            }
        })
        .collect();
    let cfg = LayoutConfig {
        node_sep: o.node_spacing,
        rank_sep: o.rank_spacing,
        edge_sep: 10.0,
    };
    let lay = layout::layout(&lnodes, &ledges, &lclusters, &cfg);
    let (lw, lhgt) = (lay.width, lay.height);
    let tf = |p: Point| match dir {
        Direction::TopBottom => p,
        Direction::BottomTop => Point {
            x: p.x,
            y: lhgt - p.y,
        },
        Direction::LeftRight => Point { x: p.y, y: p.x },
        Direction::RightLeft => Point {
            x: lhgt - p.y,
            y: p.x,
        },
    };
    let centers: Vec<Point> = lay.nodes.iter().map(|&p| tf(p)).collect();
    let (width, height) = if horizontal { (lhgt, lw) } else { (lw, lhgt) };
    let mut bounds = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: width,
        y1: height,
    };

    // Clusters: children are always created after their parent.
    let mut cluster_rects: Vec<Option<Rect>> = vec![None; fc.subgraphs.len()];
    for s in (0..fc.subgraphs.len()).rev() {
        let mut r: Option<Rect> = None;
        for (i, n) in fc.nodes.iter().enumerate() {
            if n.subgraph == Some(s) {
                let nr = Rect::around(centers[i], boxes[i].w, boxes[i].h);
                r = Some(r.map_or(nr, |r| r.union(nr)));
            }
        }
        for (c, sg) in fc.subgraphs.iter().enumerate() {
            if sg.parent == Some(s) {
                if let Some(cr) = cluster_rects[c] {
                    r = Some(r.map_or(cr, |r| r.union(cr)));
                }
            }
        }
        cluster_rects[s] = r.map(|r| Rect {
            x0: r.x0 - CLUSTER_PADDING,
            y0: r.y0 - CLUSTER_PADDING - title_height(s),
            x1: r.x1 + CLUSTER_PADDING,
            y1: r.y1 + CLUSTER_PADDING,
        });
    }
    for r in cluster_rects.iter().flatten() {
        bounds = bounds.union(*r);
    }

    let prefix = &o.id_prefix;
    let line = esc(&o.theme.line);
    let mut clusters = String::new();
    for (s, r) in cluster_rects.iter().enumerate() {
        let Some(r) = r else { continue };
        let _ = write!(
            clusters,
            r#"<g class="cluster" id="{}"><rect x="{}" y="{}" width="{}" height="{}" fill="{}" stroke="{}"/>"#,
            esc(&format!("{prefix}-{}", fc.subgraphs[s].id)),
            num(r.x0),
            num(r.y0),
            num(r.x1 - r.x0),
            num(r.y1 - r.y0),
            esc(&o.theme.cluster_fill),
            esc(&o.theme.cluster_stroke)
        );
        let title = text::measure(&fc.subgraphs[s].title, fs, lh);
        let top = Point {
            x: (r.x0 + r.x1) / 2.0,
            y: r.y0 + 4.0 + title.height / 2.0,
        };
        text_block(&mut clusters, top, &title.lines, lh, &o.theme.text);
        clusters.push_str("</g>");
    }

    let mut edges = String::new();
    let mut labels = String::new();
    let mut loops_per_node = vec![0usize; fc.nodes.len()];
    for (i, e) in fc.edges.iter().enumerate() {
        let Some((from, to)) = ends[i] else { continue };
        let (path, label_pos) = if e.from == e.to {
            if let EdgeEnd::Subgraph(_) = e.from {
                continue;
            }
            let c = centers[from];
            let b = &boxes[from];
            let k = loops_per_node[from] as f64;
            loops_per_node[from] += 1;
            let ext = 30.0 + 20.0 * k;
            let shape = fc.nodes[from].shape;
            let p0 = clip(
                shape,
                c,
                b.w,
                b.h,
                Point {
                    x: c.x + b.w,
                    y: c.y - b.h / 2.0,
                },
            );
            let p1 = clip(
                shape,
                c,
                b.w,
                b.h,
                Point {
                    x: c.x + b.w,
                    y: c.y + b.h / 2.0,
                },
            );
            let d = format!(
                "M{},{}C{},{},{},{},{},{}",
                num(p0.x),
                num(p0.y),
                num(p0.x + ext),
                num(p0.y - ext * 0.6),
                num(p1.x + ext),
                num(p1.y + ext * 0.6),
                num(p1.x),
                num(p1.y)
            );
            let right = p0.x.max(p1.x) + ext * 0.75;
            bounds = bounds.union(Rect {
                x0: c.x,
                y0: p0.y - ext * 0.5,
                x1: right,
                y1: p1.y + ext * 0.5,
            });
            let lp = label_boxes[i].as_ref().map(|t| Point {
                x: right + label_size(t).0 / 2.0 + 4.0,
                y: c.y,
            });
            (d, lp)
        } else {
            let route = &lay.edges[i];
            let mut pts: Vec<Point> = route.points.iter().map(|&p| tf(p)).collect();
            if let EdgeEnd::Subgraph(s) = e.from {
                if let Some(r) = cluster_rects[s] {
                    trim_start(&mut pts, &r);
                }
            }
            if let EdgeEnd::Subgraph(s) = e.to {
                if let Some(r) = cluster_rects[s] {
                    pts.reverse();
                    trim_start(&mut pts, &r);
                    pts.reverse();
                }
            }
            if pts.len() >= 2 {
                let n = pts.len();
                if let EdgeEnd::Node(_) = e.from {
                    pts[0] = clip(
                        fc.nodes[from].shape,
                        pts[0],
                        boxes[from].w,
                        boxes[from].h,
                        pts[1],
                    );
                }
                if let EdgeEnd::Node(_) = e.to {
                    pts[n - 1] = clip(
                        fc.nodes[to].shape,
                        pts[n - 1],
                        boxes[to].w,
                        boxes[to].h,
                        pts[n - 2],
                    );
                }
            }
            (basis_path(&pts), route.label.map(tf))
        };
        if e.stroke != Stroke::Invisible {
            let (sw, dash) = match e.stroke {
                Stroke::Thick => ("3.5", ""),
                Stroke::Dotted => ("2", r#" stroke-dasharray="3""#),
                _ => ("2", ""),
            };
            let _ = write!(
                edges,
                r#"<path d="{path}" fill="none" stroke="{line}" stroke-width="{sw}"{dash}"#
            );
            if let Some(m) = marker_ref(prefix, e.start) {
                let _ = write!(edges, r#" marker-start="{m}""#);
            }
            if let Some(m) = marker_ref(prefix, e.end) {
                let _ = write!(edges, r#" marker-end="{m}""#);
            }
            edges.push_str("/>");
        }
        if let (Some(t), Some(p)) = (&label_boxes[i], label_pos) {
            let (w, h) = label_size(t);
            let r = Rect::around(p, w, h);
            bounds = bounds.union(r);
            let _ = write!(
                labels,
                r#"<g class="edge-label"><rect x="{}" y="{}" width="{}" height="{}" fill="{}" opacity="0.9"/>"#,
                num(r.x0),
                num(r.y0),
                num(w),
                num(h),
                esc(&o.theme.edge_label_background)
            );
            text_block(&mut labels, p, &t.lines, lh, &o.theme.text);
            labels.push_str("</g>");
        }
    }

    let mut nodes = String::new();
    for (i, n) in fc.nodes.iter().enumerate() {
        let p = node_paint(fc, n, o);
        let mut attrs = format!(
            r#" fill="{}" stroke="{}" stroke-width="{}""#,
            esc(&p.fill),
            esc(&p.stroke),
            esc(&p.stroke_width)
        );
        if let Some(d) = &p.dash {
            let _ = write!(attrs, r#" stroke-dasharray="{}""#, esc(d));
        }
        let _ = write!(
            nodes,
            r#"<g class="node" id="{}">"#,
            esc(&format!("{prefix}-node-{}", n.id))
        );
        nodes.push_str(&shape_svg(
            n.shape, centers[i], boxes[i].w, boxes[i].h, &attrs,
        ));
        text_block(&mut nodes, centers[i], &boxes[i].text.lines, lh, &p.color);
        nodes.push_str("</g>");
    }

    let pad = o.padding;
    let (vx, vy) = (bounds.x0 - pad, bounds.y0 - pad);
    let (vw, vh) = (
        bounds.x1 - bounds.x0 + 2.0 * pad,
        bounds.y1 - bounds.y0 + 2.0 * pad,
    );
    let mut svg = String::new();
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" id="{id}" class="oxmaid flowchart" width="100%" viewBox="{} {} {} {}" style="max-width: {}px;" role="graphics-document document" aria-roledescription="flowchart">"#,
        num(vx),
        num(vy),
        num(vw),
        num(vh),
        num(vw),
        id = esc(prefix)
    );
    let marker = |name: &str, body: &str| {
        format!(
            r#"<marker id="{}-{name}" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" orient="auto-start-reverse">{body}</marker>"#,
            esc(prefix)
        )
    };
    svg.push_str("<defs>");
    svg.push_str(&marker(
        "point",
        &format!(r#"<path d="M0,0L10,5L0,10z" fill="{line}"/>"#),
    ));
    svg.push_str(&marker(
        "circle",
        &format!(r#"<circle cx="5" cy="5" r="4" fill="{line}"/>"#),
    ));
    svg.push_str(&marker(
        "cross",
        &format!(r#"<path d="M1,1L9,9M1,9L9,1" stroke="{line}" stroke-width="2"/>"#),
    ));
    svg.push_str("</defs>");
    let _ = write!(
        svg,
        r#"<g font-family="{}" font-size="{}">"#,
        esc(&o.font_family),
        num(fs)
    );
    for (class, body) in [
        ("clusters", clusters),
        ("edges", edges),
        ("edge-labels", labels),
        ("nodes", nodes),
    ] {
        let _ = write!(svg, r#"<g class="{class}">{body}</g>"#);
    }
    svg.push_str("</g></svg>");
    svg
}
