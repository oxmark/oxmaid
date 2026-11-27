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
    /// Enclosing cluster. Must have a lower index than the cluster itself.
    pub parent: Option<usize>,
    /// Space reserved on every side between the members and the cluster box.
    pub padding: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutEdge {
    pub from: usize,
    pub to: usize,
    /// Minimum length in ranks (at least 1).
    pub min_len: usize,
    /// Size of the edge label, if any.
    pub label: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutConfig {
    /// Gap between two nodes of the same rank.
    pub node_sep: f64,
    /// Gap between two ranks.
    pub rank_sep: f64,
    /// Gap between two edges running side by side.
    pub edge_sep: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            node_sep: 50.0,
            rank_sep: 50.0,
            edge_sep: 10.0,
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
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Node centers.
    pub nodes: Vec<Point>,
    pub edges: Vec<EdgeRoute>,
    pub width: f64,
    pub height: f64,
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
        // Parents must have a lower index, which also rules out cycles.
        self.clusters[c].parent.filter(|&p| p < c)
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

    /// Padding crossed when going from `c` up to (but excluding) `top`.
    fn padding_up_to(&self, c: Option<usize>, top: Option<usize>) -> f64 {
        self.chain(c)
            .into_iter()
            .take_while(|&k| Some(k) != top)
            .map(|k| self.clusters[k].padding)
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
    let mut x = place_x(&vs, &layers, &up, &down, cfg, &tree);
    separate_clusters(&vs, &mut x, None, cfg, &tree);
    let min = (0..nv)
        .map(|v| x[v] - vs[v].w / 2.0)
        .fold(f64::INFINITY, f64::min);
    x.iter_mut().for_each(|v| *v -= min);
    let (rank_y, height) = place_y(&vs, &layers, cfg, &tree);
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
            let mut points: Vec<Point> = std::iter::once(s)
                .chain(chains[i].iter().copied())
                .chain([t])
                .map(pos)
                .collect();
            if reversed[i] {
                points.reverse();
            }
            EdgeRoute {
                points,
                label: label_node[i].map(pos),
            }
        })
        .collect();
    Layout {
        nodes: (0..n).map(pos).collect(),
        edges: routes,
        width,
        height,
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
        tree.padding_up_to(ca, top) + tree.padding_up_to(cb, top)
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
            - it.pad;
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
            before[r0] += cl.padding;
            after[r1] += cl.padding;
        }
    }
    let half = cfg.rank_sep / 2.0;
    let mut ys = Vec::with_capacity(layers.len());
    let mut y = 0.0;
    for (r, h) in heights.iter().enumerate() {
        y = if r == 0 {
            before[0] + h / 2.0
        } else {
            y + heights[r - 1] / 2.0 + after[r - 1] + half + before[r] + h / 2.0
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
