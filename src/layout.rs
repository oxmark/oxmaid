//! Layered graph layout (Sugiyama style), standing in for dagre.
//!
//! Steps:
//! 1. break cycles by reversing DFS back edges;
//! 2. assign ranks (longest path, then pull sources towards their children);
//! 3. split long edges with dummy nodes; every edge length is doubled so an
//!    edge label can sit on its own middle rank, as dagre does;
//! 4. order each rank with barycenter sweeps, keeping the order with the
//!    fewest crossings;
//! 5. place nodes horizontally with a weighted isotonic regression (pool
//!    adjacent violators), which keeps the minimum gaps and pulls nodes
//!    towards their neighbours; dummy chains get heavy weights so long edges
//!    stay straight.
//!
//! 6. clusters (subgraphs): members of a cluster stay contiguous in every
//!    rank, gaps grow by the cluster padding at cluster borders, and a final
//!    pass shifts sibling clusters apart so their boxes never overlap.
//!
//! The layout always runs top to bottom; callers rotate the result for other
//! directions. It is not tied to flowcharts and can serve other diagrams.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutNode {
    pub width: f64,
    pub height: f64,
    /// Innermost cluster containing the node (index in the `clusters` slice).
    pub cluster: Option<usize>,
}

/// A group of nodes drawn inside a box (a flowchart subgraph).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutCluster {
    /// Enclosing cluster, in any order. Links that form a cycle are ignored.
    pub parent: Option<usize>,
    /// Space reserved on every side between the members and the cluster box.
    pub padding: f64,
    /// Extra space before the first rank (a title in a top-to-bottom layout).
    pub extra_top: f64,
    /// Extra space after the last rank (a title once the layout is flipped
    /// for a bottom-to-top diagram).
    pub extra_bottom: f64,
    /// Extra space on the left of the members (a title once the layout is
    /// rotated for a left-to-right diagram).
    pub extra_left: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutEdge {
    pub from: usize,
    pub to: usize,
    /// Minimum length in ranks (at least 1).
    pub min_len: usize,
    /// Size of the edge label, if any.
    pub label: Option<(f64, f64)>,
    /// Only constrains ranks: no route, no dummies, no influence on order.
    /// Links to subgraphs use these to keep a whole subgraph past the source.
    pub rank_only: bool,
    /// When not empty, the link leaves from whichever of these nodes is
    /// ranked last (instead of `from`) and enters whichever of `to_any` is
    /// ranked first: a link between subgraphs joins their facing members.
    pub from_any: Vec<usize>,
    pub to_any: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutConfig {
    /// Gap between two nodes of the same rank.
    pub node_sep: f64,
    /// Gap between two ranks.
    pub rank_sep: f64,
    /// Gap between two edges running side by side.
    pub edge_sep: f64,
    /// ELK spacing: a full `rank_sep` on each side of a rank holding nodes or
    /// edge labels, and half of it next to a rank of edge bends only. When
    /// false (dagre), every rank is `rank_sep / 2` from the next one.
    pub elk_ranks: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            node_sep: 50.0,
            rank_sep: 50.0,
            edge_sep: 10.0,
            elk_ranks: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EdgeRoute {
    /// From the source center to the target center. Empty for self loops,
    /// which the caller draws itself.
    pub points: Vec<Point>,
    /// Center of the label, if the edge has one.
    pub label: Option<Point>,
    /// Rank of each point.
    pub ranks: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Node centers.
    pub nodes: Vec<Point>,
    pub edges: Vec<EdgeRoute>,
    pub width: f64,
    pub height: f64,
    /// Rank of each node.
    pub node_ranks: Vec<usize>,
    /// Top and bottom of each rank (its tallest node or label).
    pub rank_bounds: Vec<(f64, f64)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Real,
    Dummy,
    Label,
}

struct VNode {
    w: f64,
    h: f64,
    kind: Kind,
    rank: usize,
    cluster: Option<usize>,
}

/// Cluster hierarchy helpers.
struct Tree<'a> {
    clusters: &'a [LayoutCluster],
}

impl Tree<'_> {
    fn parent(&self, c: usize) -> Option<usize> {
        let n = self.clusters.len();
        let p = self.clusters[c].parent.filter(|&p| p < n && p != c)?;
        // Ignore a parent link that leads back to `c` (a cycle).
        let mut k = p;
        for _ in 0..n {
            match self.clusters[k].parent.filter(|&q| q < n) {
                Some(q) if q == c => return None,
                Some(q) => k = q,
                None => return Some(p),
            }
        }
        None
    }

    /// `c` and its ancestors, innermost first.
    fn chain(&self, c: Option<usize>) -> Vec<usize> {
        std::iter::successors(c, |&k| self.parent(k)).collect()
    }

    fn lca(&self, a: Option<usize>, b: Option<usize>) -> Option<usize> {
        let pa = self.chain(a);
        self.chain(b).into_iter().find(|k| pa.contains(k))
    }

    /// The child of `c` on the way down to `x`, or `None` when `x` is `c`.
    fn child_toward(&self, c: Option<usize>, x: Option<usize>) -> Option<usize> {
        self.chain(x)
            .into_iter()
            .find(|&k| self.parent(k) == c && Some(k) != c)
    }

    /// Padding crossed when going from `c` up to (but excluding) `top`,
    /// leaving through the right side, or the left side when `left`.
    fn padding_up_to(&self, c: Option<usize>, top: Option<usize>, left: bool) -> f64 {
        self.chain(c)
            .into_iter()
            .take_while(|&k| Some(k) != top)
            .map(|k| {
                self.clusters[k].padding
                    + if left {
                        self.clusters[k].extra_left
                    } else {
                        0.0
                    }
            })
            .sum()
    }
}

/// (source, target, min length) after cycle breaking; `None` for self loops.
type EffEdge = Option<(usize, usize, usize)>;

pub fn layout(
    nodes: &[LayoutNode],
    edges: &[LayoutEdge],
    clusters: &[LayoutCluster],
    cfg: &LayoutConfig,
) -> Layout {
    let tree = Tree { clusters };
    let nodes: Vec<LayoutNode> = nodes
        .iter()
        .map(|nd| LayoutNode {
            cluster: nd.cluster.filter(|&c| c < clusters.len()),
            ..*nd
        })
        .collect();
    let n = nodes.len();
    if n == 0 {
        return Layout {
            edges: vec![EdgeRoute::default(); edges.len()],
            ..Layout::default()
        };
    }
    let valid = |e: &LayoutEdge| e.from < n && e.to < n && e.from != e.to;

    let reversed = back_edges(n, edges, &valid);
    let eff: Vec<EffEdge> = edges
        .iter()
        .zip(&reversed)
        .map(|(e, &rev)| {
            valid(e).then(|| {
                let (s, t) = if rev { (e.to, e.from) } else { (e.from, e.to) };
                (s, t, 2 * e.min_len.max(1))
            })
        })
        .collect();
    let rank = assign_ranks(n, &eff);
    // Links between groups: pick the facing members now that ranks are known.
    let eff: Vec<EffEdge> = eff
        .iter()
        .zip(edges)
        .zip(&reversed)
        .map(|((e, le), &rev)| {
            let (mut s, mut t, l) = (*e)?;
            if !rev {
                let pick = |list: &[usize], last: bool| {
                    let it = list.iter().copied().filter(|&v| v < n);
                    if last {
                        it.max_by_key(|&v| (rank[v], std::cmp::Reverse(v)))
                    } else {
                        it.min_by_key(|&v| (rank[v], v))
                    }
                };
                s = pick(&le.from_any, true).unwrap_or(s);
                t = pick(&le.to_any, false).unwrap_or(t);
                if s == t || rank[t] <= rank[s] {
                    return *e;
                }
            }
            Some((s, t, l))
        })
        .collect();

    // Virtual graph with dummy and label nodes.
    let mut vs: Vec<VNode> = nodes
        .iter()
        .zip(&rank)
        .map(|(nd, &r)| VNode {
            w: nd.width,
            h: nd.height,
            kind: Kind::Real,
            rank: r,
            cluster: nd.cluster,
        })
        .collect();
    let mut chains: Vec<Vec<usize>> = vec![Vec::new(); edges.len()];
    let mut label_node: Vec<Option<usize>> = vec![None; edges.len()];
    let mut vedges = Vec::new();
    for (i, e) in eff.iter().enumerate() {
        let Some((s, t, _)) = *e else { continue };
        if edges[i].rank_only {
            continue;
        }
        let (rs, rt) = (rank[s], rank[t]);
        let mid = rs + (rt - rs) / 2;
        let cluster = tree.lca(nodes[s].cluster, nodes[t].cluster);
        let mut prev = s;
        for r in rs + 1..rt {
            let id = vs.len();
            match (r == mid, edges[i].label) {
                (true, Some((w, h))) => {
                    vs.push(VNode {
                        w,
                        h,
                        kind: Kind::Label,
                        rank: r,
                        cluster,
                    });
                    label_node[i] = Some(id);
                }
                _ => vs.push(VNode {
                    w: 0.0,
                    h: 0.0,
                    kind: Kind::Dummy,
                    rank: r,
                    cluster,
                }),
            }
            vedges.push((prev, id));
            chains[i].push(id);
            prev = id;
        }
        vedges.push((prev, t));
    }
    let nv = vs.len();
    let mut up = vec![Vec::new(); nv];
    let mut down = vec![Vec::new(); nv];
    for &(a, b) in &vedges {
        down[a].push(b);
        up[b].push(a);
    }

    let layers = order(&vs, &up, &down, n, &tree);
    let mut x = if cfg.elk_ranks {
        brandes_koepf(&vs, &layers, &up, &down, cfg, &tree)
    } else {
        place_x(&vs, &layers, &up, &down, cfg, &tree)
    };
    separate_clusters(&vs, &mut x, None, cfg, &tree);
    let min = (0..nv)
        .map(|v| x[v] - vs[v].w / 2.0)
        .fold(f64::INFINITY, f64::min);
    x.iter_mut().for_each(|v| *v -= min);
    let (rank_y, height) = place_y(&vs, &layers, cfg, &tree);
    let rank_bounds: Vec<(f64, f64)> = layers
        .iter()
        .zip(&rank_y)
        .map(|(l, &y)| {
            let h = l.iter().map(|&v| vs[v].h).fold(0.0, f64::max);
            (y - h / 2.0, y + h / 2.0)
        })
        .collect();
    let pos = |v: usize| Point {
        x: x[v],
        y: rank_y[vs[v].rank],
    };

    let width = (0..nv).map(|v| x[v] + vs[v].w / 2.0).fold(0.0, f64::max);
    let routes = eff
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let Some((s, t, _)) = *e else {
                return EdgeRoute::default();
            };
            if edges[i].rank_only {
                return EdgeRoute::default();
            }
            let path: Vec<usize> = std::iter::once(s)
                .chain(chains[i].iter().copied())
                .chain([t])
                .collect();
            let mut points: Vec<Point> = path.iter().map(|&v| pos(v)).collect();
            let mut ranks: Vec<usize> = path.iter().map(|&v| vs[v].rank).collect();
            if reversed[i] {
                points.reverse();
                ranks.reverse();
            }
            EdgeRoute {
                points,
                label: label_node[i].map(pos),
                ranks,
            }
        })
        .collect();
    Layout {
        nodes: (0..n).map(pos).collect(),
        edges: routes,
        width,
        height,
        node_ranks: vs[..n].iter().map(|v| v.rank).collect(),
        rank_bounds,
    }
}

/// Marks the edges that close a cycle (iterative DFS, input order).
fn back_edges(n: usize, edges: &[LayoutEdge], valid: &dyn Fn(&LayoutEdge) -> bool) -> Vec<bool> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in edges.iter().enumerate() {
        if valid(e) {
            out[e.from].push(i);
        }
    }
    let mut state = vec![0u8; n]; // 0 unvisited, 1 on stack, 2 done
    let mut rev = vec![false; edges.len()];
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        let mut stack = vec![(root, 0usize)];
        while let Some(top) = stack.last_mut() {
            let v = top.0;
            if let Some(&ei) = out[v].get(top.1) {
                top.1 += 1;
                let w = edges[ei].to;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => rev[ei] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    rev
}

fn assign_ranks(n: usize, eff: &[EffEdge]) -> Vec<usize> {
    let mut indeg = vec![0usize; n];
    let mut outs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    let mut ins: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for &(s, t, l) in eff.iter().flatten() {
        indeg[t] += 1;
        outs[s].push((t, l));
        ins[t].push((s, l));
    }
    let mut queue: VecDeque<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut topo = Vec::with_capacity(n);
    while let Some(v) = queue.pop_front() {
        topo.push(v);
        for &(t, _) in &outs[v] {
            indeg[t] -= 1;
            if indeg[t] == 0 {
                queue.push_back(t);
            }
        }
    }
    let mut rank = vec![0usize; n];
    for &v in &topo {
        for &(s, l) in &ins[v] {
            rank[v] = rank[v].max(rank[s] + l);
        }
    }
    // Move nodes with more children than parents down next to their children.
    for &v in topo.iter().rev() {
        if outs[v].len() > ins[v].len() {
            if let Some(r) = outs[v].iter().map(|&(t, l)| rank[t] - l).min() {
                rank[v] = rank[v].max(r);
            }
        }
    }
    let min = rank.iter().copied().min().unwrap_or(0);
    rank.iter().map(|r| r - min).collect()
}

fn order(
    vs: &[VNode],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    n_real: usize,
    tree: &Tree,
) -> Vec<Vec<usize>> {
    let max_rank = vs.iter().map(|v| v.rank).max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    let mut seen = vec![false; vs.len()];
    let mut starts: Vec<usize> = (0..n_real).collect();
    starts.sort_by_key(|&v| vs[v].rank);
    for s in starts.into_iter().chain(n_real..vs.len()) {
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            if std::mem::replace(&mut seen[v], true) {
                continue;
            }
            layers[vs[v].rank].push(v);
            stack.extend(down[v].iter().rev().copied().filter(|&c| !seen[c]));
        }
    }
    let mut pos = vec![0usize; vs.len()];
    for layer in &layers {
        for (i, &v) in layer.iter().enumerate() {
            pos[v] = i;
        }
    }
    let mut best = layers.clone();
    let mut best_crossings = crossings(&layers, down, &pos);
    for iter in 0..24 {
        if best_crossings == 0 {
            break;
        }
        if iter % 2 == 0 {
            for layer in layers.iter_mut().skip(1) {
                sort_layer(layer, up, &mut pos, vs, tree);
            }
        } else {
            for r in (0..layers.len() - 1).rev() {
                sort_layer(&mut layers[r], down, &mut pos, vs, tree);
            }
        }
        let c = crossings(&layers, down, &pos);
        if c < best_crossings {
            best_crossings = c;
            best = layers.clone();
        }
    }
    if !tree.clusters.is_empty() {
        consistent_cluster_order(&mut best, vs, tree);
    }
    best
}

/// Orders `items` hierarchically: siblings (nodes and child clusters of `c`)
/// are sorted by key, and each cluster's members are expanded in place, so
/// that cluster members are contiguous.
fn arrange(
    c: Option<usize>,
    items: &[usize],
    vs: &[VNode],
    tree: &Tree,
    node_key: &dyn Fn(usize) -> f64,
    group_key: &dyn Fn(usize, &[usize]) -> f64,
) -> Vec<usize> {
    let mut direct = Vec::new();
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for &v in items {
        match tree.child_toward(c, vs[v].cluster) {
            None => direct.push(v),
            Some(k) => match groups.iter_mut().find(|g| g.0 == k) {
                Some(g) => g.1.push(v),
                None => groups.push((k, vec![v])),
            },
        }
    }
    let mut keyed: Vec<(f64, Vec<usize>)> =
        direct.into_iter().map(|v| (node_key(v), vec![v])).collect();
    for (k, members) in groups {
        let key = group_key(k, &members);
        keyed.push((
            key,
            arrange(Some(k), &members, vs, tree, node_key, group_key),
        ));
    }
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    keyed.into_iter().flat_map(|(_, v)| v).collect()
}

/// Gives sibling clusters the same left-to-right order in every rank.
fn consistent_cluster_order(layers: &mut [Vec<usize>], vs: &[VNode], tree: &Tree) {
    let mut norm = vec![0.0; vs.len()];
    for layer in layers.iter() {
        for (i, &v) in layer.iter().enumerate() {
            norm[v] = (i as f64 + 0.5) / layer.len() as f64;
        }
    }
    let mut sum = vec![(0.0, 0usize); tree.clusters.len()];
    for (v, n) in vs.iter().enumerate() {
        for k in tree.chain(n.cluster) {
            sum[k].0 += norm[v];
            sum[k].1 += 1;
        }
    }
    let global: Vec<f64> = sum
        .iter()
        .map(|&(s, c)| if c > 0 { s / c as f64 } else { 0.0 })
        .collect();
    for layer in layers.iter_mut() {
        *layer = arrange(None, layer, vs, tree, &|v| norm[v], &|k, _| global[k]);
    }
}

fn sort_layer(
    layer: &mut [usize],
    neighbours: &[Vec<usize>],
    pos: &mut [usize],
    vs: &[VNode],
    tree: &Tree,
) {
    let bary = |v: usize| {
        let ns = &neighbours[v];
        if ns.is_empty() {
            pos[v] as f64
        } else {
            ns.iter().map(|&u| pos[u] as f64).sum::<f64>() / ns.len() as f64
        }
    };
    let sorted = arrange(None, layer, vs, tree, &bary, &|_, members| {
        members.iter().map(|&v| bary(v)).sum::<f64>() / members.len() as f64
    });
    for (i, &v) in sorted.iter().enumerate() {
        layer[i] = v;
        pos[v] = i;
    }
}

fn crossings(layers: &[Vec<usize>], down: &[Vec<usize>], pos: &[usize]) -> usize {
    let mut total = 0;
    for layer in layers {
        let es: Vec<(usize, usize)> = layer
            .iter()
            .flat_map(|&u| down[u].iter().map(move |&v| (pos[u], pos[v])))
            .collect();
        for i in 0..es.len() {
            for j in i + 1..es.len() {
                let ((a, b), (c, d)) = (es[i], es[j]);
                if (a < c && b > d) || (a > c && b < d) {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Margin kept free at both ends of a node side when spreading ports.
pub const PORT_MARGIN: f64 = 10.0;

/// Offsets from the node center of the ports of `v` towards `ns` (its
/// neighbours in one adjacent rank), in the same order as `ns`: links are
/// spread evenly over the side, in the order of the nodes they lead to.
fn port_offsets(vs: &[VNode], v: usize, ns: &[usize], slot: &[usize]) -> Vec<f64> {
    let k = ns.len();
    let mut out = vec![0.0; k];
    if k < 2 {
        return out;
    }
    let mut idx: Vec<usize> = (0..k).collect();
    idx.sort_by_key(|&i| (slot[ns[i]], i));
    let w = vs[v].w;
    let usable = (w - 2.0 * PORT_MARGIN).max(0.0);
    for (j, &i) in idx.iter().enumerate() {
        out[i] = -w / 2.0 + PORT_MARGIN.min(w / 2.0) + usable * (j as f64 + 1.0) / (k as f64 + 1.0);
    }
    out
}

/// Brandes-Köpf coordinate assignment, as ELK runs it: blocks of nodes are
/// aligned port to port towards the previous or the next rank, scanning from
/// the left or the right; each of the four results is compacted, and the
/// final position is the mean of the two median ones (ELK's `BALANCED`).
fn brandes_koepf(
    vs: &[VNode],
    layers: &[Vec<usize>],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    cfg: &LayoutConfig,
    tree: &Tree,
) -> Vec<f64> {
    let nv = vs.len();
    let mut slot = vec![0usize; nv];
    for layer in layers {
        for (i, &v) in layer.iter().enumerate() {
            slot[v] = i;
        }
    }
    let top_ports: Vec<Vec<f64>> = (0..nv)
        .map(|v| port_offsets(vs, v, &up[v], &slot))
        .collect();
    let bottom_ports: Vec<Vec<f64>> = (0..nv)
        .map(|v| port_offsets(vs, v, &down[v], &slot))
        .collect();
    // Port of `v` on the link to `u`, on the top side or the bottom side.
    let port = |v: usize, u: usize, top: bool| {
        let (ns, offs) = if top {
            (&up[v], &top_ports[v])
        } else {
            (&down[v], &bottom_ports[v])
        };
        ns.iter().position(|&x| x == u).map_or(0.0, |i| offs[i])
    };
    let inner = |v: usize| vs[v].kind != Kind::Real;
    // Type 1 conflicts: a segment between two dummies wins over any other
    // segment crossing it, which is marked (upper end, lower end).
    let mut marked: std::collections::HashSet<(usize, usize)> = Default::default();
    for i in 1..layers.len().saturating_sub(1) {
        let (upper, lower) = (&layers[i], &layers[i + 1]);
        let (mut k0, mut l) = (0usize, 0usize);
        for (l1, &v) in lower.iter().enumerate() {
            let w = if inner(v) {
                up[v].iter().copied().find(|&u| inner(u))
            } else {
                None
            };
            if l1 + 1 == lower.len() || w.is_some() {
                let k1 = w.map_or(upper.len().saturating_sub(1), |w| slot[w]);
                while l <= l1 {
                    let x = lower[l];
                    for &u in &up[x] {
                        let k = slot[u];
                        if (k < k0 || k > k1) && !(inner(u) && inner(x)) {
                            marked.insert((u, x));
                        }
                    }
                    l += 1;
                }
                k0 = k1;
            }
        }
    }

    let mut results: Vec<(bool, Vec<f64>)> = Vec::with_capacity(4);
    for (to_prev, from_left) in [(true, true), (true, false), (false, true), (false, false)] {
        // Mirrored coordinates when scanning from the right.
        let sign = if from_left { 1.0 } else { -1.0 };
        let mut order: Vec<Vec<usize>> = layers.to_vec();
        if !to_prev {
            order.reverse();
        }
        if !from_left {
            order.iter_mut().for_each(|l| l.reverse());
        }
        let pos = |v: usize| {
            if from_left {
                slot[v]
            } else {
                layers[vs[v].rank].len() - 1 - slot[v]
            }
        };
        // Vertical alignment; `shift` is a node's offset from its block root.
        let mut root: Vec<usize> = (0..nv).collect();
        let mut align: Vec<usize> = (0..nv).collect();
        let mut shift = vec![0.0f64; nv];
        for layer in order.iter().skip(1) {
            let mut r: Option<usize> = None;
            for &v in layer {
                let mut ns: Vec<usize> = if to_prev {
                    up[v].clone()
                } else {
                    down[v].clone()
                };
                if ns.is_empty() {
                    continue;
                }
                ns.sort_by_key(|&u| pos(u));
                ns.dedup();
                let d = ns.len();
                for m in [(d - 1) / 2, d / 2] {
                    if align[v] != v {
                        break;
                    }
                    let u = ns[m];
                    let conflict = if to_prev {
                        marked.contains(&(u, v))
                    } else {
                        marked.contains(&(v, u))
                    };
                    if !conflict && r.map_or(true, |r| r < pos(u)) {
                        align[u] = v;
                        root[v] = root[u];
                        align[v] = root[v];
                        // Line up the two ends of the link.
                        let (pu, pv) = (port(u, v, !to_prev), port(v, u, to_prev));
                        shift[v] = shift[u] + sign * (pu - pv);
                        r = Some(pos(u));
                    }
                }
            }
        }
        // Horizontal compaction on the block graph: as far as possible
        // towards the scan side, then back where blocks have room to spare.
        let sep = |a: usize, b: usize| {
            // `a` comes before `b` in scan order.
            if from_left {
                separation(vs, a, b, cfg, tree)
            } else {
                separation(vs, b, a, cfg, tree)
            }
        };
        let mut preds: Vec<Vec<(usize, f64)>> = vec![Vec::new(); nv];
        let mut succs: Vec<Vec<(usize, f64)>> = vec![Vec::new(); nv];
        for layer in &order {
            for w in layer.windows(2) {
                let (a, b) = (root[w[0]], root[w[1]]);
                let d = sep(w[0], w[1]) + shift[w[0]] - shift[w[1]];
                preds[b].push((a, d));
                succs[a].push((b, d));
            }
        }
        let roots: Vec<usize> = (0..nv).filter(|&v| root[v] == v).collect();
        let topo = topo_order(&roots, &preds, &succs);
        let mut bx = vec![0.0f64; nv];
        for &b in &topo {
            bx[b] = preds[b].iter().map(|&(a, d)| bx[a] + d).fold(0.0, f64::max);
        }
        for &b in topo.iter().rev() {
            let room = succs[b]
                .iter()
                .map(|&(c, d)| bx[c] - d)
                .fold(f64::INFINITY, f64::min);
            if room.is_finite() && !preds[b].is_empty() {
                bx[b] = bx[b].max(room);
            }
        }
        let x: Vec<f64> = (0..nv).map(|v| sign * (bx[root[v]] + shift[v])).collect();
        results.push((from_left, x));
    }

    // Balance: align every result with the narrowest one (by its left side
    // for left scans, its right side otherwise), then take the median.
    let extent = |x: &[f64]| {
        let lo = (0..nv)
            .map(|v| x[v] - vs[v].w / 2.0)
            .fold(f64::INFINITY, f64::min);
        let hi = (0..nv)
            .map(|v| x[v] + vs[v].w / 2.0)
            .fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    };
    let bounds: Vec<(f64, f64)> = results.iter().map(|(_, x)| extent(x)).collect();
    let narrowest = (0..results.len())
        .min_by(|&a, &b| (bounds[a].1 - bounds[a].0).total_cmp(&(bounds[b].1 - bounds[b].0)))
        .unwrap_or(0);
    for (i, (left, x)) in results.iter_mut().enumerate() {
        let d = if *left {
            bounds[narrowest].0 - bounds[i].0
        } else {
            bounds[narrowest].1 - bounds[i].1
        };
        x.iter_mut().for_each(|v| *v += d);
    }
    (0..nv)
        .map(|v| {
            let mut xs = [
                results[0].1[v],
                results[1].1[v],
                results[2].1[v],
                results[3].1[v],
            ];
            xs.sort_by(f64::total_cmp);
            (xs[1] + xs[2]) / 2.0
        })
        .collect()
}

/// Topological order of the block graph (it is acyclic: blocks never cross).
fn topo_order(
    roots: &[usize],
    preds: &[Vec<(usize, f64)>],
    succs: &[Vec<(usize, f64)>],
) -> Vec<usize> {
    let mut indeg: std::collections::HashMap<usize, usize> =
        roots.iter().map(|&r| (r, preds[r].len())).collect();
    let mut stack: Vec<usize> = roots
        .iter()
        .copied()
        .filter(|r| preds[*r].is_empty())
        .rev()
        .collect();
    let mut out = Vec::with_capacity(roots.len());
    while let Some(b) = stack.pop() {
        out.push(b);
        for &(c, _) in &succs[b] {
            let d = indeg.get_mut(&c).expect("successors are roots");
            *d -= 1;
            if *d == 0 {
                stack.push(c);
            }
        }
    }
    out
}

fn separation(vs: &[VNode], a: usize, b: usize, cfg: &LayoutConfig, tree: &Tree) -> f64 {
    let gap = match (vs[a].kind == Kind::Real, vs[b].kind == Kind::Real) {
        (true, true) => cfg.node_sep,
        (false, false) => cfg.edge_sep,
        _ => (cfg.node_sep + cfg.edge_sep) / 2.0,
    };
    let (ca, cb) = (vs[a].cluster, vs[b].cluster);
    let borders = if ca == cb {
        0.0
    } else {
        let top = tree.lca(ca, cb);
        tree.padding_up_to(ca, top, false) + tree.padding_up_to(cb, top, true)
    };
    (vs[a].w + vs[b].w) / 2.0 + gap + borders
}

fn edge_weight(vs: &[VNode], a: usize, b: usize) -> f64 {
    match (vs[a].kind == Kind::Real, vs[b].kind == Kind::Real) {
        (true, true) => 1.0,
        (false, false) => 8.0,
        _ => 2.0,
    }
}

fn place_x(
    vs: &[VNode],
    layers: &[Vec<usize>],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    cfg: &LayoutConfig,
    tree: &Tree,
) -> Vec<f64> {
    let mut x = vec![0.0; vs.len()];
    for layer in layers {
        let mut cx = 0.0;
        for (i, &v) in layer.iter().enumerate() {
            if i > 0 {
                cx += separation(vs, layer[i - 1], v, cfg, tree);
            }
            x[v] = cx;
        }
    }
    let pass = |x: &mut Vec<f64>, layer: &[usize], use_up: bool, use_down: bool| {
        if layer.is_empty() {
            return;
        }
        let mut desired = Vec::with_capacity(layer.len());
        let mut weights = Vec::with_capacity(layer.len());
        for &v in layer {
            let (mut sw, mut sx) = (0.0, 0.0);
            let lists = [(use_up, &up[v]), (use_down, &down[v])];
            for (on, ns) in lists {
                if on {
                    for &u in ns {
                        let w = edge_weight(vs, u, v);
                        sw += w;
                        sx += w * x[u];
                    }
                }
            }
            if sw > 0.0 {
                desired.push(sx / sw);
                weights.push(sw);
            } else {
                desired.push(x[v]);
                weights.push(1.0);
            }
        }
        let seps: Vec<f64> = (0..layer.len())
            .map(|i| {
                if i == 0 {
                    0.0
                } else {
                    separation(vs, layer[i - 1], layer[i], cfg, tree)
                }
            })
            .collect();
        for (&v, nx) in layer.iter().zip(isotonic(&desired, &weights, &seps)) {
            x[v] = nx;
        }
    };
    let last = layers.len().saturating_sub(1);
    for _ in 0..10 {
        for layer in &layers[1..] {
            pass(&mut x, layer, true, false);
        }
        for layer in layers[..last].iter().rev() {
            pass(&mut x, layer, false, true);
        }
    }
    for _ in 0..4 {
        for layer in layers {
            pass(&mut x, layer, true, true);
        }
    }
    x
}

/// Shifts the children of cluster `c` (its own nodes and its sub-clusters)
/// to the right until no two of them overlap on a shared rank range.
fn separate_clusters(
    vs: &[VNode],
    x: &mut [f64],
    c: Option<usize>,
    cfg: &LayoutConfig,
    tree: &Tree,
) {
    let children: Vec<usize> = (0..tree.clusters.len())
        .filter(|&k| tree.parent(k) == c)
        .collect();
    for &k in &children {
        separate_clusters(vs, x, Some(k), cfg, tree);
    }
    struct Item {
        members: Vec<usize>,
        pad: f64,
        pad_left: f64,
        r0: usize,
        r1: usize,
        x0: f64,
        x1: f64,
    }
    let mut items: Vec<Item> = Vec::new();
    for (v, n) in vs.iter().enumerate() {
        if n.cluster == c {
            items.push(Item {
                members: vec![v],
                pad: 0.0,
                pad_left: 0.0,
                r0: 0,
                r1: 0,
                x0: 0.0,
                x1: 0.0,
            });
        }
    }
    for &k in &children {
        let members: Vec<usize> = (0..vs.len())
            .filter(|&v| tree.chain(vs[v].cluster).contains(&k))
            .collect();
        if !members.is_empty() {
            items.push(Item {
                members,
                pad: tree.clusters[k].padding,
                pad_left: tree.clusters[k].padding + tree.clusters[k].extra_left,
                r0: 0,
                r1: 0,
                x0: 0.0,
                x1: 0.0,
            });
        }
    }
    for it in &mut items {
        it.r0 = it.members.iter().map(|&v| vs[v].rank).min().unwrap_or(0);
        it.r1 = it.members.iter().map(|&v| vs[v].rank).max().unwrap_or(0);
        it.x0 = it
            .members
            .iter()
            .map(|&v| x[v] - vs[v].w / 2.0)
            .fold(f64::INFINITY, f64::min)
            - it.pad_left;
        it.x1 = it
            .members
            .iter()
            .map(|&v| x[v] + vs[v].w / 2.0)
            .fold(f64::NEG_INFINITY, f64::max)
            + it.pad;
    }
    items.sort_by(|a, b| (a.x0 + a.x1).total_cmp(&(b.x0 + b.x1)));
    for i in 0..items.len() {
        let (done, rest) = items.split_at_mut(i);
        let it = &mut rest[0];
        let shift = done
            .iter()
            .filter(|o| o.r0 <= it.r1 && it.r0 <= o.r1 && (o.pad > 0.0 || it.pad > 0.0))
            .map(|o| o.x1 + cfg.edge_sep - it.x0)
            .fold(0.0, f64::max);
        if shift > 0.0 {
            for &v in &it.members {
                x[v] += shift;
            }
            it.x0 += shift;
            it.x1 += shift;
        }
    }
}

/// Weighted least squares fit of `x` to `desired` under `x[i] >= x[i-1] + seps[i]`
/// (pool adjacent violators on the gap-shifted values).
fn isotonic(desired: &[f64], weights: &[f64], seps: &[f64]) -> Vec<f64> {
    let n = desired.len();
    let mut offset = vec![0.0; n];
    for i in 1..n {
        offset[i] = offset[i - 1] + seps[i];
    }
    // Blocks of (sum of weights, weighted sum, count).
    let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
    for i in 0..n {
        blocks.push((weights[i], weights[i] * (desired[i] - offset[i]), 1));
        while blocks.len() >= 2 {
            let b = blocks[blocks.len() - 1];
            let a = blocks[blocks.len() - 2];
            if a.1 / a.0 > b.1 / b.0 {
                blocks.pop();
                *blocks.last_mut().unwrap() = (a.0 + b.0, a.1 + b.1, a.2 + b.2);
            } else {
                break;
            }
        }
    }
    let mut out = Vec::with_capacity(n);
    for (sw, swx, count) in blocks {
        for _ in 0..count {
            out.push(swx / sw + offset[out.len()]);
        }
    }
    out
}

fn place_y(
    vs: &[VNode],
    layers: &[Vec<usize>],
    cfg: &LayoutConfig,
    tree: &Tree,
) -> (Vec<f64>, f64) {
    let heights: Vec<f64> = layers
        .iter()
        .map(|l| l.iter().map(|&v| vs[v].h).fold(0.0, f64::max))
        .collect();
    // Extra room above the first rank and below the last rank of each cluster.
    let mut before = vec![0.0; layers.len()];
    let mut after = vec![0.0; layers.len()];
    for (k, cl) in tree.clusters.iter().enumerate() {
        let ranks = vs
            .iter()
            .filter(|v| tree.chain(v.cluster).contains(&k))
            .map(|v| v.rank);
        let (r0, r1) = ranks.fold((usize::MAX, 0), |(a, b), r| (a.min(r), b.max(r)));
        if r0 <= r1 {
            before[r0] += cl.padding + cl.extra_top;
            after[r1] += cl.padding + cl.extra_bottom;
        }
    }
    let half = cfg.rank_sep / 2.0;
    let gap = |r: usize| {
        if cfg.elk_ranks && heights[r - 1] > 0.0 && heights[r] > 0.0 {
            cfg.rank_sep
        } else {
            half
        }
    };
    let mut ys = Vec::with_capacity(layers.len());
    let mut y = 0.0;
    for (r, h) in heights.iter().enumerate() {
        y = if r == 0 {
            before[0] + h / 2.0
        } else {
            y + heights[r - 1] / 2.0 + after[r - 1] + gap(r) + before[r] + h / 2.0
        };
        ys.push(y);
    }
    let total =
        y + heights.last().copied().unwrap_or(0.0) / 2.0 + after.last().copied().unwrap_or(0.0);
    (ys, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> LayoutNode {
        LayoutNode {
            width: 40.0,
            height: 20.0,
            cluster: None,
        }
    }

    fn edge(from: usize, to: usize) -> LayoutEdge {
        LayoutEdge {
            from,
            to,
            min_len: 1,
            label: None,
            rank_only: false,
            from_any: Vec::new(),
            to_any: Vec::new(),
        }
    }

    #[test]
    fn chain_is_vertical_and_straight() {
        let l = layout(
            &[node(), node(), node()],
            &[edge(0, 1), edge(1, 2)],
            &[],
            &LayoutConfig::default(),
        );
        assert!(l.nodes[0].y < l.nodes[1].y && l.nodes[1].y < l.nodes[2].y);
        assert_eq!(l.nodes[0].x, l.nodes[2].x);
    }

    #[test]
    fn cycles_and_self_loops_do_not_panic() {
        let l = layout(
            &[node(), node()],
            &[edge(0, 1), edge(1, 0), edge(1, 1)],
            &[],
            &LayoutConfig::default(),
        );
        assert_eq!(l.edges[0].points.first(), Some(&l.nodes[0]));
        assert_eq!(l.edges[1].points.first(), Some(&l.nodes[1]));
        assert!(l.edges[2].points.is_empty());
    }

    #[test]
    fn same_rank_nodes_do_not_overlap() {
        let nodes = vec![node(); 4];
        let edges = [edge(0, 1), edge(0, 2), edge(0, 3)];
        let l = layout(&nodes, &edges, &[], &LayoutConfig::default());
        let mut xs: Vec<f64> = (1..4).map(|i| l.nodes[i].x).collect();
        xs.sort_by(f64::total_cmp);
        assert!(xs.windows(2).all(|w| w[1] - w[0] >= 40.0 + 50.0 - 1e-6));
    }

    #[test]
    fn sibling_clusters_do_not_overlap() {
        // Two clusters of two stacked nodes, plus edges that tempt the
        // ordering to interleave them.
        let mk = |c| LayoutNode {
            width: 40.0,
            height: 20.0,
            cluster: Some(c),
        };
        let nodes = [mk(0), mk(0), mk(1), mk(1)];
        let edges = [edge(0, 1), edge(2, 3), edge(0, 3), edge(2, 1)];
        let clusters = [LayoutCluster {
            parent: None,
            padding: 15.0,
            extra_top: 0.0,
            extra_bottom: 0.0,
            extra_left: 0.0,
        }; 2];
        let l = layout(&nodes, &edges, &clusters, &LayoutConfig::default());
        let span = |a: usize, b: usize| {
            (
                l.nodes[a].x.min(l.nodes[b].x) - 35.0,
                l.nodes[a].x.max(l.nodes[b].x) + 35.0,
            )
        };
        let (a, b) = (span(0, 1), span(2, 3));
        assert!(a.1 <= b.0 || b.1 <= a.0, "{a:?} {b:?}");
    }

    #[test]
    fn isotonic_keeps_gaps() {
        let out = isotonic(&[5.0, 0.0], &[1.0, 1.0], &[0.0, 10.0]);
        assert!((out[1] - out[0] - 10.0).abs() < 1e-9);
        assert!((out[0] - (-2.5)).abs() < 1e-9);
    }
}
