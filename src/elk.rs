//! ELK-style geometry shared by the diagrams drawn with Mermaid's `neo`
//! look: the crate's layered layout run with ELK's spacing, then orthogonal
//! links with ports spread along node sides, in diagram space.

use std::collections::HashMap;
use std::fmt::Write;

use crate::flowchart::Direction;
use crate::layout::{self, LayoutCluster, LayoutConfig, LayoutEdge, LayoutNode, Point};

/// Corners of orthogonal links are rounded over this length.
const CORNER: f64 = 5.0 * std::f64::consts::SQRT_2;
/// Ports keep this distance from the corners of a node side.
const PORT_MARGIN: f64 = 10.0;

/// A link end on a box side: position along the side, route index, and
/// whether it is the link's start.
type Port = (f64, usize, bool);

/// A link end: a node or a cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    Node(usize),
    Cluster(usize),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GNode {
    /// Size as drawn.
    pub w: f64,
    pub h: f64,
    /// Innermost cluster.
    pub cluster: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GEdge {
    pub from: End,
    pub to: End,
    pub min_len: usize,
    /// Label size as drawn.
    pub label: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GCluster {
    pub parent: Option<usize>,
    /// Title size.
    pub title: (f64, f64),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Spacing {
    pub node: f64,
    pub rank: f64,
    pub edge: f64,
    pub cluster_padding: f64,
    /// Room for a cluster title, on top of its height.
    pub title_gap: f64,
    /// Padding around the drawing.
    pub margin: f64,
}

/// Half extent of a node outline: `(node, width, height, vertical, offset
/// from the center across the link, toward the positive side)`.
pub(crate) type Extent<'a> = &'a dyn Fn(usize, f64, f64, bool, f64, bool) -> f64;

pub(crate) struct Graph<'a> {
    pub dir: Direction,
    pub nodes: &'a [GNode],
    pub edges: &'a [GEdge],
    pub clusters: &'a [GCluster],
    pub spacing: Spacing,
    pub extent: Extent<'a>,
}

/// Everything in diagram space.
pub(crate) struct Geometry {
    pub node_rects: Vec<Rect>,
    pub cluster_rects: Vec<Option<Rect>>,
    pub label_rects: Vec<Option<Rect>>,
    /// Link polylines; `None` for links that could not be laid out.
    pub polylines: Vec<Option<Vec<Point>>>,
    /// Bounds of the whole drawing: x0, y0, x1, y1.
    pub bounds: (f64, f64, f64, f64),
}

/// Cluster depth (0 for a top-level cluster).
pub(crate) fn depth(clusters: &[GCluster], mut s: usize) -> usize {
    let mut d = 0;
    while let Some(p) = clusters[s].parent {
        d += 1;
        s = p;
    }
    d
}

/// Formats a number like JavaScript's `Number.prototype.toString`.
pub(crate) fn js(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let v = if v == 0.0 { 0.0 } else { v };
    format!("{v}")
}

pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Lays out the graph and routes its links.
pub(crate) fn geometry(g: &Graph) -> Geometry {
    let sp = g.spacing;
    let dir = g.dir;
    let horizontal = dir.is_horizontal();
    let swap = |w: f64, h: f64| if horizontal { (h, w) } else { (w, h) };
    // Layout, top to bottom (sizes swapped for left-to-right diagrams).
    let lnodes: Vec<LayoutNode> = g
        .nodes
        .iter()
        .map(|n| {
            let (width, height) = swap(n.w, n.h);
            LayoutNode {
                width,
                height,
                cluster: n.cluster,
            }
        })
        .collect();
    let title_room = |s: usize| {
        let h = g.clusters[s].title.1;
        if h > 0.0 {
            h + sp.title_gap
        } else {
            0.0
        }
    };
    let lclusters: Vec<LayoutCluster> = (0..g.clusters.len())
        .map(|s| {
            let extra = title_room(s);
            LayoutCluster {
                parent: g.clusters[s].parent,
                padding: sp.cluster_padding,
                extra_top: if matches!(dir, Direction::TopBottom) {
                    extra
                } else {
                    0.0
                },
                extra_bottom: if matches!(dir, Direction::BottomTop) {
                    extra
                } else {
                    0.0
                },
                extra_left: if horizontal { extra } else { 0.0 },
            }
        })
        .collect();
    let sg_members: Vec<Vec<usize>> = (0..g.clusters.len()).map(|s| members(g, s)).collect();
    let layout_end = |end: End, source: bool| match end {
        End::Node(i) => Some(i),
        End::Cluster(s) => {
            if source {
                sg_members[s].last().copied()
            } else {
                sg_members[s].first().copied()
            }
        }
    };
    let ends: Vec<Option<(usize, usize)>> = g
        .edges
        .iter()
        .map(|e| Some((layout_end(e.from, true)?, layout_end(e.to, false)?)))
        .collect();
    let mut ledges: Vec<LayoutEdge> = g
        .edges
        .iter()
        .zip(&ends)
        .map(|(e, ends)| {
            let (from, to) = ends.unwrap_or((0, 0));
            LayoutEdge {
                from,
                to,
                min_len: e.min_len,
                label: e.label.map(|(w, h)| swap(w.ceil(), h.ceil())),
                rank_only: false,
                from_any: match e.from {
                    End::Cluster(s) => sg_members[s].clone(),
                    End::Node(_) => Vec::new(),
                },
                to_any: match e.to {
                    End::Cluster(s) => sg_members[s].clone(),
                    End::Node(_) => Vec::new(),
                },
            }
        })
        .collect();
    // A link to or from a subgraph places the whole subgraph past its other
    // end, as ELK does with the subgraph as one compound node.
    let all_of = |end: End| match end {
        End::Node(i) => vec![i],
        End::Cluster(s) => sg_members[s].clone(),
    };
    for e in g.edges {
        if matches!((e.from, e.to), (End::Node(_), End::Node(_))) || e.from == e.to {
            continue;
        }
        let (froms, tos) = (all_of(e.from), all_of(e.to));
        for &from in &froms {
            for &to in &tos {
                if from != to {
                    ledges.push(LayoutEdge {
                        from,
                        to,
                        min_len: e.min_len,
                        label: None,
                        rank_only: true,
                        from_any: Vec::new(),
                        to_any: Vec::new(),
                    });
                }
            }
        }
    }
    let cfg = LayoutConfig {
        node_sep: sp.node,
        rank_sep: sp.rank,
        edge_sep: sp.edge,
        elk_ranks: true,
    };
    let lay = layout::layout(&lnodes, &ledges, &lclusters, &cfg);

    // Boxes in layout space.
    let node_box: Vec<Rect> = lay
        .nodes
        .iter()
        .zip(&lnodes)
        .map(|(c, n)| Rect::around(*c, n.width, n.height))
        .collect();
    let mut cluster_box: Vec<Option<Rect>> = vec![None; g.clusters.len()];
    // Inner clusters first: a parent closes after its children.
    let mut order: Vec<usize> = (0..g.clusters.len()).collect();
    let depth = |mut s: usize| {
        let mut d = 0;
        while let Some(p) = g.clusters[s].parent {
            d += 1;
            s = p;
        }
        d
    };
    order.sort_by_key(|&s| std::cmp::Reverse(depth(s)));
    for &s in &order {
        let mut rects: Vec<Rect> = sg_members[s].iter().map(|&i| node_box[i]).collect();
        rects.extend(
            (0..g.clusters.len())
                .filter(|&c| g.clusters[c].parent == Some(s))
                .filter_map(|c| cluster_box[c]),
        );
        for (k, (e, route)) in g.edges.iter().zip(&lay.edges).enumerate() {
            let inside = |end: End| match end {
                End::Node(i) => sg_members[s].contains(&i),
                End::Cluster(_) => false,
            };
            if let (Some(p), Some((lw, lh))) = (route.label, e.label) {
                if inside(e.from) && inside(e.to) && (lw > 0.0 || lh > 0.0) {
                    let (w, h) = ledges[k].label.unwrap_or((0.0, 0.0));
                    rects.push(Rect::around(p, w, h));
                }
            }
        }
        if rects.is_empty() {
            continue;
        }
        let x0 = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
        let y0 = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
        let x1 = rects
            .iter()
            .map(|r| r.x + r.w)
            .fold(f64::NEG_INFINITY, f64::max);
        let y1 = rects
            .iter()
            .map(|r| r.y + r.h)
            .fold(f64::NEG_INFINITY, f64::max);
        let c = &lclusters[s];
        let mut r = Rect {
            x: x0 - c.padding - c.extra_left,
            y: y0 - c.padding - c.extra_top,
            w: x1 - x0 + 2.0 * c.padding + c.extra_left,
            h: y1 - y0 + 2.0 * c.padding + c.extra_top + c.extra_bottom,
        };
        // Wide enough for the title.
        let (tw, _) = swap(g.clusters[s].title.0 + 2.0 * sp.cluster_padding, 0.0);
        if horizontal {
            let min_h = g.clusters[s].title.0 + 2.0 * sp.cluster_padding;
            if r.h < min_h {
                r.y -= (min_h - r.h) / 2.0;
                r.h = min_h;
            }
        } else if r.w < tw {
            r.x -= (tw - r.w) / 2.0;
            r.w = tw;
        }
        cluster_box[s] = Some(r);
    }

    // Routes in layout space.
    let boxed = |end: End| match end {
        End::Node(i) => Some(Boxed::Node(i)),
        End::Cluster(s) => cluster_box[s].map(|_| Boxed::Cluster(s)),
    };
    let rect_of = |b: Boxed| match b {
        Boxed::Node(i) => node_box[i],
        Boxed::Cluster(s) => cluster_box[s].expect("only laid out clusters are boxed"),
    };
    struct Route {
        from: Boxed,
        to: Boxed,
        /// Inner points (dummies, label), whether they sit on a rank with
        /// nodes or labels, and that rank's top and bottom.
        inner: Vec<(Point, bool, (f64, f64))>,
        start_down: bool,
        end_down: bool,
    }
    let rank_full = |r: usize| lay.rank_bounds.get(r).is_some_and(|(a, b)| b - a > 1e-9);
    let mut routes: Vec<Option<Route>> = Vec::with_capacity(g.edges.len());
    for (e, route) in g.edges.iter().zip(&lay.edges) {
        let (Some(from), Some(to)) = (boxed(e.from), boxed(e.to)) else {
            routes.push(None);
            continue;
        };
        if from == to || route.points.len() < 2 {
            routes.push(Some(Route {
                from,
                to,
                inner: Vec::new(),
                start_down: true,
                end_down: true,
            }));
            continue;
        }
        let (fr, tr) = (rect_of(from), rect_of(to));
        let n = route.points.len();
        let inner: Vec<(Point, bool, (f64, f64))> = (1..n - 1)
            .map(|i| {
                let r = route.ranks[i];
                (
                    route.points[i],
                    rank_full(r),
                    lay.rank_bounds.get(r).copied().unwrap_or_default(),
                )
            })
            .filter(|(p, _, _)| !(matches!(from, Boxed::Cluster(_)) && fr.contains(*p)))
            .filter(|(p, _, _)| !(matches!(to, Boxed::Cluster(_)) && tr.contains(*p)))
            .collect();
        let after_start = inner.first().map_or(tr.center(), |p| p.0);
        let before_end = inner.last().map_or(fr.center(), |p| p.0);
        routes.push(Some(Route {
            from,
            to,
            start_down: after_start.y > fr.center().y,
            end_down: before_end.y < tr.center().y,
            inner,
        }));
    }

    // Ports: links sharing a side of a box are spread along it.
    let mut sides: HashMap<(Boxed, bool), Vec<Port>> = HashMap::new();
    for (i, r) in routes.iter().enumerate() {
        let Some(r) = r else { continue };
        if r.from == r.to {
            continue;
        }
        let (fr, tr) = (rect_of(r.from), rect_of(r.to));
        let toward_start = r.inner.first().map_or(tr.center(), |p| p.0);
        let toward_end = r.inner.last().map_or(fr.center(), |p| p.0);
        // Bottom side of the source when going down; top side otherwise.
        sides
            .entry((r.from, r.start_down))
            .or_default()
            .push((toward_start.x, i, true));
        sides
            .entry((r.to, !r.end_down))
            .or_default()
            .push((toward_end.x, i, false));
    }
    let mut port_x: HashMap<(usize, bool), f64> = HashMap::new();
    for ((b, _), list) in &mut sides {
        list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let r = rect_of(*b);
        let k = list.len() as f64;
        let usable = (r.w - 2.0 * PORT_MARGIN).max(0.0);
        for (j, (_, edge, is_start)) in list.iter().enumerate() {
            let x = if list.len() == 1 {
                match b {
                    // A subgraph is wide: meet the link where it comes from.
                    Boxed::Cluster(_) => list[0].0.clamp(
                        r.x + PORT_MARGIN,
                        (r.x + r.w - PORT_MARGIN).max(r.x + PORT_MARGIN),
                    ),
                    Boxed::Node(_) => r.x + r.w / 2.0,
                }
            } else {
                r.x + PORT_MARGIN + usable * (j as f64 + 1.0) / (k + 1.0)
            };
            port_x.insert((*edge, *is_start), x);
        }
    }

    // Orthogonal polylines, still in layout space.
    let mut lines: Vec<Option<(Vec<Point>, Vec<bool>)>> = Vec::with_capacity(routes.len());
    for (i, r) in routes.iter().enumerate() {
        let Some(r) = r else {
            lines.push(None);
            continue;
        };
        if r.from == r.to {
            // A small loop off the left side, as ELK draws self loops.
            let b = rect_of(r.from);
            let (x, cy) = (b.x, b.y + b.h / 2.0);
            let pts = vec![
                Point { x, y: cy - 3.5 },
                Point {
                    x: x - 20.0,
                    y: cy - 3.5,
                },
                Point {
                    x: x - 20.0,
                    y: cy + 3.5,
                },
                Point { x, y: cy + 3.5 },
            ];
            let keep = vec![false; pts.len()];
            lines.push(Some((pts, keep)));
            continue;
        }
        let (fr, tr) = (rect_of(r.from), rect_of(r.to));
        let sx = port_x.get(&(i, true)).copied().unwrap_or(fr.center().x);
        let tx = port_x.get(&(i, false)).copied().unwrap_or(tr.center().x);
        let start = Point {
            x: sx,
            y: if r.start_down { fr.y + fr.h } else { fr.y },
        };
        let end = Point {
            x: tx,
            y: if r.end_down { tr.y } else { tr.y + tr.h },
        };
        // (point, on a full rank, top and bottom of what it stands for)
        let mut stops: Vec<(Point, bool, (f64, f64))> = vec![(start, true, (start.y, start.y))];
        stops.extend(r.inner.iter().copied());
        stops.push((end, true, (end.y, end.y)));
        let mut pts = vec![start];
        let mut keep = vec![false];
        for w in stops.windows(2) {
            let ((c, c_full, c_span), (q, q_full, q_span)) = (w[0], w[1]);
            if (c.x - q.x).abs() > 1e-6 {
                if !q_full {
                    pts.push(Point { x: c.x, y: q.y });
                    keep.push(false);
                } else if !c_full {
                    pts.push(Point { x: q.x, y: c.y });
                    keep.push(false);
                } else {
                    // Middle of the free space between the two ranks.
                    let y = if q.y > c.y {
                        (c_span.1 + q_span.0) / 2.0
                    } else {
                        (c_span.0 + q_span.1) / 2.0
                    };
                    pts.push(Point { x: c.x, y });
                    pts.push(Point { x: q.x, y });
                    keep.extend([false, false]);
                }
            }
            pts.push(q);
            keep.push(false);
        }
        lines.push(Some((pts, keep)));
    }

    // To diagram space: rotate, then shift so the drawing starts at MARGIN.
    let tf = |p: Point| match dir {
        Direction::TopBottom => p,
        Direction::BottomTop => Point { x: p.x, y: -p.y },
        Direction::LeftRight => Point { x: p.y, y: p.x },
        Direction::RightLeft => Point { x: -p.y, y: p.x },
    };
    let tf_rect = |r: Rect| {
        let (a, b) = (
            tf(Point { x: r.x, y: r.y }),
            tf(Point {
                x: r.x + r.w,
                y: r.y + r.h,
            }),
        );
        Rect {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    };
    let node_rects: Vec<Rect> = node_box.iter().map(|r| tf_rect(*r)).collect();
    let cluster_rects: Vec<Option<Rect>> = cluster_box.iter().map(|r| r.map(tf_rect)).collect();
    let label_rects: Vec<Option<Rect>> = lay
        .edges
        .iter()
        .zip(&ledges)
        .map(|(r, e)| {
            let (p, (w, h)) = (r.label?, e.label?);
            Some(tf_rect(Rect::around(p, w, h)))
        })
        .collect();
    let mut polylines: Vec<Option<Vec<Point>>> = lines
        .into_iter()
        .map(|l| {
            l.map(|(pts, keep)| {
                let pts: Vec<Point> = pts.into_iter().map(tf).collect();
                simplify(pts, &keep)
            })
        })
        .collect();
    let bounds = node_rects
        .iter()
        .copied()
        .chain(cluster_rects.iter().flatten().copied())
        .chain(label_rects.iter().flatten().copied())
        .chain(polylines.iter().flatten().flatten().map(|p| Rect {
            x: p.x,
            y: p.y,
            w: 0.0,
            h: 0.0,
        }));
    let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
    for r in bounds {
        minx = minx.min(r.x);
        miny = miny.min(r.y);
    }
    if !minx.is_finite() {
        minx = 0.0;
        miny = 0.0;
    }
    let (dx, dy) = (sp.margin - minx, sp.margin - miny);
    let mv = |p: Point| Point {
        x: p.x + dx,
        y: p.y + dy,
    };
    let mv_rect = |r: Rect| Rect {
        x: r.x + dx,
        y: r.y + dy,
        ..r
    };
    let node_rects: Vec<Rect> = node_rects.into_iter().map(mv_rect).collect();
    let cluster_rects: Vec<Option<Rect>> =
        cluster_rects.into_iter().map(|r| r.map(mv_rect)).collect();
    let label_rects: Vec<Option<Rect>> = label_rects.into_iter().map(|r| r.map(mv_rect)).collect();
    for line in polylines.iter_mut().flatten() {
        line.iter_mut().for_each(|p| *p = mv(*p));
    }

    // Links leave and enter a node where its outline is, not its box.
    for (k, line) in polylines.iter_mut().enumerate() {
        let Some(line) = line else { continue };
        if line.len() < 2 {
            continue;
        }
        let e = &g.edges[k];
        let attach = |line: &mut Vec<Point>, at_start: bool, i: usize| {
            let r = node_rects[i];
            let (p, q) = if at_start {
                (line[0], line[1])
            } else {
                (line[line.len() - 1], line[line.len() - 2])
            };
            let c = r.center();
            let vertical = (p.x - q.x).abs() < 1e-6;
            let (extent, full) = if vertical {
                (
                    (g.extent)(i, r.w, r.h, true, p.x - c.x, p.y > c.y),
                    r.h / 2.0,
                )
            } else {
                (
                    (g.extent)(i, r.w, r.h, false, p.y - c.y, p.x > c.x),
                    r.w / 2.0,
                )
            };
            if full - extent > 1e-6 {
                let o = if vertical {
                    Point {
                        x: p.x,
                        y: c.y + extent * (p.y - c.y).signum(),
                    }
                } else {
                    Point {
                        x: c.x + extent * (p.x - c.x).signum(),
                        y: p.y,
                    }
                };
                if at_start {
                    line.insert(0, o);
                } else {
                    line.push(o);
                }
            }
        };
        if e.from != e.to {
            if let End::Node(i) = e.from {
                attach(line, true, i);
            }
            if let End::Node(i) = e.to {
                attach(line, false, i);
            }
        }
    }

    // Bounds.
    let mut all: Vec<Rect> = node_rects.clone();
    all.extend(cluster_rects.iter().flatten());
    all.extend(label_rects.iter().flatten());
    all.extend(polylines.iter().flatten().flatten().map(|p| Rect {
        x: p.x,
        y: p.y,
        w: 0.0,
        h: 0.0,
    }));
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for r in &all {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.w);
        y1 = y1.max(r.y + r.h);
    }
    if !x0.is_finite() {
        (x0, y0, x1, y1) = (0.0, 0.0, 0.0, 0.0);
    }
    Geometry {
        node_rects,
        cluster_rects,
        label_rects,
        polylines,
        bounds: (x0, y0, x1, y1),
    }
}

/// Every node inside cluster `s`, nested ones included.
fn members(g: &Graph, s: usize) -> Vec<usize> {
    let inside = |mut sub: Option<usize>| {
        while let Some(k) = sub {
            if k == s {
                return true;
            }
            sub = g.clusters[k].parent;
        }
        false
    };
    (0..g.nodes.len())
        .filter(|&i| inside(g.nodes[i].cluster))
        .collect()
}

/// An end of a link once laid out: a node or a subgraph box.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Boxed {
    Node(usize),
    Cluster(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub(crate) fn around(c: Point, w: f64, h: f64) -> Self {
        Self {
            x: c.x - w / 2.0,
            y: c.y - h / 2.0,
            w,
            h,
        }
    }
    pub(crate) fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2.0,
            y: self.y + self.h / 2.0,
        }
    }
    pub(crate) fn contains(&self, p: Point) -> bool {
        p.x > self.x + 1e-6
            && p.x < self.x + self.w - 1e-6
            && p.y > self.y + 1e-6
            && p.y < self.y + self.h - 1e-6
    }
}

// ---------------------------------------------------------------------------
// Links

/// `d` of an orthogonal polyline with rounded corners.
pub(crate) fn rounded_path(pts: &[Point]) -> String {
    let mut d = String::new();
    let Some(first) = pts.first() else { return d };
    let _ = write!(d, "M{},{}", js(first.x), js(first.y));
    for i in 1..pts.len() {
        let p = pts[i];
        if i + 1 < pts.len() {
            let (a, b) = (pts[i - 1], pts[i + 1]);
            let (l1, l2) = (dist(a, p), dist(p, b));
            let cross = (p.x - a.x) * (b.y - p.y) - (p.y - a.y) * (b.x - p.x);
            if l1 > 1e-9 && l2 > 1e-9 && cross.abs() > 1e-9 {
                let r = CORNER.min(l1 / 2.0).min(l2 / 2.0);
                let p1 = Point {
                    x: p.x - (p.x - a.x) / l1 * r,
                    y: p.y - (p.y - a.y) / l1 * r,
                };
                let p2 = Point {
                    x: p.x + (b.x - p.x) / l2 * r,
                    y: p.y + (b.y - p.y) / l2 * r,
                };
                let _ = write!(
                    d,
                    "L{},{}Q{},{} {},{}",
                    js(p1.x),
                    js(p1.y),
                    js(p.x),
                    js(p.y),
                    js(p2.x),
                    js(p2.y)
                );
                continue;
            }
        }
        let _ = write!(d, "L{},{}", js(p.x), js(p.y));
    }
    d
}

/// Length of [`rounded_path`].
pub(crate) fn rounded_length(pts: &[Point]) -> f64 {
    let mut len: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
    for i in 1..pts.len().saturating_sub(1) {
        let (a, p, b) = (pts[i - 1], pts[i], pts[i + 1]);
        let (l1, l2) = (dist(a, p), dist(p, b));
        let cross = (p.x - a.x) * (b.y - p.y) - (p.y - a.y) * (b.x - p.x);
        if l1 > 1e-9 && l2 > 1e-9 && cross.abs() > 1e-9 {
            let r = CORNER.min(l1 / 2.0).min(l2 / 2.0);
            let p1 = Point {
                x: p.x - (p.x - a.x) / l1 * r,
                y: p.y - (p.y - a.y) / l1 * r,
            };
            let p2 = Point {
                x: p.x + (b.x - p.x) / l2 * r,
                y: p.y + (b.y - p.y) / l2 * r,
            };
            len += quad_length(p1, p, p2) - 2.0 * r;
        }
    }
    len
}

pub(crate) fn quad_length(a: Point, c: Point, b: Point) -> f64 {
    let at = |t: f64| Point {
        x: (1.0 - t).powi(2) * a.x + 2.0 * (1.0 - t) * t * c.x + t * t * b.x,
        y: (1.0 - t).powi(2) * a.y + 2.0 * (1.0 - t) * t * c.y + t * t * b.y,
    };
    let steps = 32;
    (0..steps)
        .map(|i| {
            dist(
                at(i as f64 / steps as f64),
                at((i + 1) as f64 / steps as f64),
            )
        })
        .sum()
}

/// `curve: basis` through the points (d3's `curveBasis`).
pub(crate) fn basis_path(pts: &[Point]) -> String {
    let mut d = String::new();
    if pts.len() < 3 {
        for (i, p) in pts.iter().enumerate() {
            let _ = write!(
                d,
                "{}{},{}",
                if i == 0 { "M" } else { "L" },
                js(p.x),
                js(p.y)
            );
        }
        return d;
    }
    let p = pts;
    let _ = write!(d, "M{},{}", js(p[0].x), js(p[0].y));
    let _ = write!(
        d,
        "L{},{}",
        js((5.0 * p[0].x + p[1].x) / 6.0),
        js((5.0 * p[0].y + p[1].y) / 6.0)
    );
    for i in 1..p.len() - 1 {
        let (a, b, c) = (p[i - 1], p[i], p[i + 1]);
        let _ = write!(
            d,
            "C{},{},{},{},{},{}",
            js((2.0 * a.x + b.x) / 3.0),
            js((2.0 * a.y + b.y) / 3.0),
            js((a.x + 2.0 * b.x) / 3.0),
            js((a.y + 2.0 * b.y) / 3.0),
            js((a.x + 4.0 * b.x + c.x) / 6.0),
            js((a.y + 4.0 * b.y + c.y) / 6.0)
        );
    }
    let (a, b) = (p[p.len() - 2], p[p.len() - 1]);
    let _ = write!(
        d,
        "C{},{},{},{},{},{}",
        js((2.0 * a.x + b.x) / 3.0),
        js((2.0 * a.y + b.y) / 3.0),
        js((a.x + 2.0 * b.x) / 3.0),
        js((a.y + 2.0 * b.y) / 3.0),
        js(b.x),
        js(b.y)
    );
    d
}

pub(crate) fn dist(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Moves `p` towards `toward` by `by`.
pub(crate) fn pull(p: Point, toward: Point, by: f64) -> Point {
    let l = dist(p, toward);
    if l <= by || l < 1e-9 {
        return p;
    }
    Point {
        x: p.x + (toward.x - p.x) / l * by,
        y: p.y + (toward.y - p.y) / l * by,
    }
}

/// `data-points`: the points as JSON.
pub(crate) fn points_json(pts: &[Point]) -> String {
    let items: Vec<String> = pts
        .iter()
        .map(|p| format!(r#"{{"x":{},"y":{}}}"#, js(p.x), js(p.y)))
        .collect();
    format!("[{}]", items.join(","))
}

/// Removes repeated points and middle points of straight runs.
pub(crate) fn simplify(pts: Vec<Point>, keep: &[bool]) -> Vec<Point> {
    let mut out: Vec<(Point, bool)> = Vec::with_capacity(pts.len());
    for (p, &k) in pts.into_iter().zip(keep) {
        if out.last().is_some_and(|(q, _)| dist(*q, p) < 1e-6) {
            continue;
        }
        while out.len() >= 2 {
            let (a, _) = out[out.len() - 2];
            let (b, kb) = out[out.len() - 1];
            let cross = (b.x - a.x) * (p.y - b.y) - (b.y - a.y) * (p.x - b.x);
            // Collinear: a straight run, or a run that doubles back on itself.
            if cross.abs() < 1e-6 && !kb {
                out.pop();
            } else {
                break;
            }
        }
        out.push((p, k));
    }
    out.into_iter().map(|(p, _)| p).collect()
}
