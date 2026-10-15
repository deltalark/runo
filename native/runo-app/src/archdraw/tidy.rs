//! Making a diagram easy to read: a layered layout that keeps connectors from crossing,
//! connector ends spread along a box's side, parallel connectors pulled apart, and a
//! score of how tangled a diagram looks, so Beautify can keep whichever version reads best.

use super::geom::{Pt, Side, Wire};
use super::model::Rect;
use std::collections::HashMap;

// ---- Layered layout

/// Room between neighbouring boxes in a layer, and between layers.
const GAP_ACROSS: f32 = 64.;
const GAP_ALONG: f32 = 88.;
/// Room kept for a connector passing through a layer on its way further down.
const GAP_CHANNEL: f32 = 26.;
/// Room between separate groups of connected boxes.
const GAP_GROUPS: f32 = 120.;

/// A layered layout: callers above (or left of) what they call, boxes in each layer ordered
/// so as few connectors as possible cross, connectors that skip layers given a channel of
/// their own, and each box lined up with what it connects to. Separate groups sit side by
/// side, and boxes with no connectors in a grid after them. Returns top-left corners.
pub(crate) fn layered(
    ids: &[String],
    sizes: &HashMap<String, Pt>,
    links: &[(String, String)],
    origin: Pt,
    horizontal: bool,
) -> HashMap<String, Pt> {
    let n = ids.len();
    let index: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for (a, b) in links {
        if let (Some(&i), Some(&j)) = (index.get(a.as_str()), index.get(b.as_str())) {
            if i != j && !edges.contains(&(i, j)) {
                edges.push((i, j));
            }
        }
    }
    let size = |v: usize| sizes.get(&ids[v]).copied().unwrap_or((180., 90.));
    let across = |v: usize| if horizontal { size(v).1 } else { size(v).0 };
    let along = |v: usize| if horizontal { size(v).0 } else { size(v).1 };

    // Groups of boxes joined by connectors.
    let mut root: Vec<usize> = (0..n).collect();
    fn find(root: &mut [usize], v: usize) -> usize {
        let mut r = v;
        while root[r] != r {
            r = root[r];
        }
        let mut c = v;
        while root[c] != r {
            let next = root[c];
            root[c] = r;
            c = next;
        }
        r
    }
    for &(a, b) in &edges {
        let (ra, rb) = (find(&mut root, a), find(&mut root, b));
        if ra != rb {
            root[ra] = rb;
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for v in 0..n {
        let r = find(&mut root, v);
        groups.entry(r).or_default().push(v);
    }
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    // Biggest first; ties in the order the shapes were given, so the result is stable.
    groups.sort_by_key(|g| (std::cmp::Reverse(g.len()), g[0]));
    let (lone, groups): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
        groups.into_iter().partition(|g| g.len() == 1);

    let place = |a: f32, l: f32| {
        if horizontal {
            (origin.0 + l, origin.1 + a)
        } else {
            (origin.0 + a, origin.1 + l)
        }
    };
    let mut out = HashMap::new();
    let mut cursor = 0.;
    for g in &groups {
        let inner: Vec<(usize, usize)> = edges
            .iter()
            .copied()
            .filter(|(a, b)| g.contains(a) && g.contains(b))
            .collect();
        let (pos, width) = layered_group(g, &inner, &across, &along);
        for (v, (a, l)) in pos {
            out.insert(ids[v].clone(), place(cursor + a, l));
        }
        cursor += width + GAP_GROUPS;
    }
    // Boxes on their own: a grid, roughly square.
    let lone: Vec<usize> = lone.into_iter().flatten().collect();
    if !lone.is_empty() {
        let cols = ((lone.len() as f32).sqrt().ceil() as usize).max(1);
        let mut l = 0.;
        for row in lone.chunks(cols) {
            let mut a = cursor;
            let thick = row.iter().map(|&v| along(v)).fold(0., f32::max);
            for &v in row {
                out.insert(ids[v].clone(), place(a, l));
                a += across(v) + GAP_ACROSS;
            }
            l += thick + GAP_ALONG;
        }
    }
    out
}

/// Lay out one connected group. Returns each box's top-left corner as (across, along),
/// and the width of the group across the layers.
fn layered_group(
    nodes: &[usize],
    edges: &[(usize, usize)],
    across: &dyn Fn(usize) -> f32,
    along: &dyn Fn(usize) -> f32,
) -> (HashMap<usize, Pt>, f32) {
    let m = nodes.len();
    let local: HashMap<usize, usize> = nodes.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let mut out_e: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut indeg0 = vec![0usize; m];
    for &(a, b) in edges {
        let (i, j) = (local[&a], local[&b]);
        if !out_e[i].contains(&j) {
            out_e[i].push(j);
            indeg0[j] += 1;
        }
    }
    // Break cycles by turning round the links that go back up, starting the walk from
    // boxes nothing points at so they end up on top.
    let mut state = vec![0u8; m];
    let mut dag: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut starts: Vec<usize> = (0..m).filter(|&v| indeg0[v] == 0).collect();
    starts.extend((0..m).filter(|&v| indeg0[v] != 0));
    for s in starts {
        if state[s] != 0 {
            continue;
        }
        // An explicit stack: big diagrams would overflow a recursive walk.
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        state[s] = 1;
        while let Some(&mut (v, ref mut k)) = stack.last_mut() {
            if *k < out_e[v].len() {
                let w = out_e[v][*k];
                *k += 1;
                match state[w] {
                    1 => {
                        if !dag[w].contains(&v) {
                            dag[w].push(v);
                        }
                    }
                    0 => {
                        dag[v].push(w);
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    _ => {
                        if !dag[v].contains(&w) {
                            dag[v].push(w);
                        }
                    }
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    // Layers: the longest path from the top, then boxes that only call down are pulled
    // to just above what they call, so they don't sit far from it.
    let mut indeg = vec![0usize; m];
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); m];
    for (v, outs) in dag.iter().enumerate() {
        for &w in outs {
            indeg[w] += 1;
            ins[w].push(v);
        }
    }
    let mut topo: Vec<usize> = (0..m).filter(|&v| indeg[v] == 0).collect();
    let mut qi = 0;
    let mut left = indeg.clone();
    while qi < topo.len() {
        let v = topo[qi];
        qi += 1;
        for &w in &dag[v] {
            left[w] -= 1;
            if left[w] == 0 {
                topo.push(w);
            }
        }
    }
    let mut layer = vec![0usize; m];
    for &v in &topo {
        for &w in &dag[v] {
            layer[w] = layer[w].max(layer[v] + 1);
        }
    }
    for &v in topo.iter().rev() {
        if ins[v].is_empty() && !dag[v].is_empty() {
            let lowest = dag[v].iter().map(|&w| layer[w]).min().unwrap_or(1);
            layer[v] = lowest.saturating_sub(1).max(layer[v]);
        }
    }
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;

    // A point on each layer a connector passes through, so it gets a channel of its own.
    // Items below m are boxes; the rest are those points.
    let mut item_layer: Vec<usize> = layer.clone();
    let mut downs: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut ups: Vec<Vec<usize>> = vec![Vec::new(); m];
    for v in 0..m {
        for &w in &dag[v] {
            let mut prev = v;
            for l in layer[v] + 1..layer[w] {
                let d = item_layer.len();
                item_layer.push(l);
                downs.push(Vec::new());
                ups.push(Vec::new());
                downs[prev].push(d);
                ups[d].push(prev);
                prev = d;
            }
            downs[prev].push(w);
            ups[w].push(prev);
        }
    }
    let total = item_layer.len();
    let is_box = |x: usize| x < m;

    // A first order: the order a walk from the top meets each item.
    let mut seen = vec![usize::MAX; total];
    let mut count = 0;
    let mut order_walk = |start: usize, seen: &mut Vec<usize>| {
        let mut stack = vec![start];
        while let Some(x) = stack.pop() {
            if seen[x] != usize::MAX {
                continue;
            }
            seen[x] = count;
            count += 1;
            for &y in downs[x].iter().rev() {
                stack.push(y);
            }
        }
    };
    let roots: Vec<usize> = (0..m).filter(|&v| ups[v].is_empty()).collect();
    for v in roots {
        order_walk(v, &mut seen);
    }
    for x in 0..total {
        order_walk(x, &mut seen);
    }
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for (x, &l) in item_layer.iter().enumerate() {
        layers[l].push(x);
    }
    for l in &mut layers {
        l.sort_by_key(|&x| seen[x]);
    }

    // Order each layer to cut crossings: sweeps by the median of the neighbours, then
    // swapping neighbours where that helps, keeping the best order found.
    let mut pos = vec![0usize; total];
    let set_pos = |layers: &[Vec<usize>], pos: &mut [usize]| {
        for l in layers {
            for (i, &x) in l.iter().enumerate() {
                pos[x] = i;
            }
        }
    };
    set_pos(&layers, &mut pos);
    let mut best = layers.clone();
    let mut best_cross = all_crossings(&layers, &downs, &pos);
    for round in 0..24 {
        if best_cross == 0 {
            break;
        }
        let down = round % 2 == 0;
        let range: Vec<usize> = if down {
            (1..depth).collect()
        } else {
            (0..depth.saturating_sub(1)).rev().collect()
        };
        for li in range {
            let nb = if down { &ups } else { &downs };
            let mut keyed: Vec<(f32, usize, usize)> = layers[li]
                .iter()
                .enumerate()
                .map(|(i, &x)| {
                    let mut p: Vec<usize> = nb[x].iter().map(|&y| pos[y]).collect();
                    p.sort_unstable();
                    let k = match p.len() {
                        0 => i as f32,
                        n if n % 2 == 1 => p[n / 2] as f32,
                        n => (p[n / 2 - 1] + p[n / 2]) as f32 / 2.,
                    };
                    (k, i, x)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            layers[li] = keyed.into_iter().map(|(_, _, x)| x).collect();
            set_pos(&layers, &mut pos);
        }
        transpose(&mut layers, &ups, &downs, &mut pos);
        let c = all_crossings(&layers, &downs, &pos);
        if c < best_cross {
            best_cross = c;
            best = layers.clone();
        }
    }
    let layers = best;
    set_pos(&layers, &mut pos);

    // Across: each item as close as it can be to the middle of what it connects to,
    // keeping the order and the gaps. Two passes that push the opposite ways, averaged,
    // keep the result centred instead of drifting to one side.
    let width_of = |x: usize| if is_box(x) { across(nodes[x]) } else { 0. };
    let sep = |a: usize, b: usize| {
        let gap = match (is_box(a), is_box(b)) {
            (true, true) => GAP_ACROSS,
            (false, false) => GAP_CHANNEL,
            _ => GAP_CHANNEL + 10.,
        };
        (width_of(a) + width_of(b)) / 2. + gap
    };
    let mut x = vec![0f32; total];
    for l in &layers {
        let mut c = 0.;
        for (i, &it) in l.iter().enumerate() {
            if i > 0 {
                c += sep(l[i - 1], it);
            }
            x[it] = c;
        }
    }
    let median = |v: &[f32]| -> Option<f32> {
        if v.is_empty() {
            return None;
        }
        let mut v = v.to_vec();
        v.sort_by(f32::total_cmp);
        let n = v.len();
        Some(if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.
        })
    };
    for round in 0..12 {
        let down = round % 2 == 0;
        let range: Vec<usize> = if down {
            (0..depth).collect()
        } else {
            (0..depth).rev().collect()
        };
        for li in range {
            let l = &layers[li];
            if l.is_empty() {
                continue;
            }
            let want: Vec<f32> = l
                .iter()
                .map(|&it| {
                    let up: Vec<f32> = ups[it].iter().map(|&y| x[y]).collect();
                    let dn: Vec<f32> = downs[it].iter().map(|&y| x[y]).collect();
                    // Points of a passing connector follow it exactly, to keep it straight.
                    let (first, second) = if down { (&up, &dn) } else { (&dn, &up) };
                    match (median(first), median(second)) {
                        (Some(a), Some(b)) if is_box(it) => (a + b) / 2.,
                        (Some(a), _) => a,
                        (None, Some(b)) => b,
                        (None, None) => x[it],
                    }
                })
                .collect();
            let k = l.len();
            let mut lp = vec![0f32; k];
            let mut rp = vec![0f32; k];
            for i in 0..k {
                lp[i] = if i == 0 {
                    want[0]
                } else {
                    want[i].max(lp[i - 1] + sep(l[i - 1], l[i]))
                };
            }
            for i in (0..k).rev() {
                rp[i] = if i == k - 1 {
                    want[k - 1]
                } else {
                    want[i].min(rp[i + 1] - sep(l[i], l[i + 1]))
                };
            }
            for i in 0..k {
                x[l[i]] = (lp[i] + rp[i]) / 2.;
            }
        }
    }
    let lo = (0..total)
        .map(|it| x[it] - width_of(it) / 2.)
        .fold(f32::INFINITY, f32::min);
    let hi = (0..total)
        .map(|it| x[it] + width_of(it) / 2.)
        .fold(f32::NEG_INFINITY, f32::max);

    // Along: layers stacked, each as thick as its biggest box, with more room where many
    // connectors pass between two layers.
    let mut out = HashMap::new();
    let mut main = 0.;
    for (li, l) in layers.iter().enumerate() {
        let thick = l
            .iter()
            .filter(|&&it| is_box(it))
            .map(|&it| along(nodes[it]))
            .fold(0., f32::max);
        for &it in l {
            if is_box(it) {
                let v = nodes[it];
                let a = x[it] - across(v) / 2. - lo;
                out.insert(v, (a, main + (thick - along(v)) / 2.));
            }
        }
        let between = l.iter().map(|&it| downs[it].len()).sum::<usize>();
        let _ = li;
        main += thick + GAP_ALONG + (between.min(12) as f32) * 4.;
    }
    (out, (hi - lo).max(0.))
}

/// Crossings between every pair of neighbouring layers.
fn all_crossings(layers: &[Vec<usize>], downs: &[Vec<usize>], pos: &[usize]) -> usize {
    layers
        .windows(2)
        .map(|w| crossings_between(&w[0], downs, pos))
        .sum()
}

/// Crossings among the connectors leaving one layer for the next.
fn crossings_between(upper: &[usize], downs: &[Vec<usize>], pos: &[usize]) -> usize {
    let mut pairs: Vec<(usize, usize)> = upper
        .iter()
        .flat_map(|&a| downs[a].iter().map(move |&b| (pos[a], pos[b])))
        .collect();
    pairs.sort_unstable();
    let mut ends: Vec<usize> = pairs.iter().map(|p| p.1).collect();
    inversions(&mut ends)
}

/// Pairs out of order, counted while merge-sorting.
fn inversions(v: &mut [usize]) -> usize {
    let n = v.len();
    if n < 2 {
        return 0;
    }
    let mid = n / 2;
    let mut count = inversions(&mut v[..mid]) + inversions(&mut v[mid..]);
    let mut merged = Vec::with_capacity(n);
    let (mut i, mut j) = (0, mid);
    while i < mid && j < n {
        if v[i] <= v[j] {
            merged.push(v[i]);
            i += 1;
        } else {
            merged.push(v[j]);
            count += mid - i;
            j += 1;
        }
    }
    merged.extend_from_slice(&v[i..mid]);
    merged.extend_from_slice(&v[j..n]);
    v.copy_from_slice(&merged);
    count
}

/// Swap neighbours in a layer while that cuts the crossings around the layer.
fn transpose(
    layers: &mut [Vec<usize>],
    ups: &[Vec<usize>],
    downs: &[Vec<usize>],
    pos: &mut [usize],
) {
    // Crossings between two items' connectors to one side, in their current order.
    let pair = |a: usize, b: usize, nb: &[Vec<usize>], pos: &[usize]| -> usize {
        let mut c = 0;
        for &p in &nb[a] {
            for &q in &nb[b] {
                if pos[p] > pos[q] {
                    c += 1;
                }
            }
        }
        c
    };
    for _ in 0..4 {
        let mut better = false;
        for l in layers.iter_mut() {
            for i in 0..l.len().saturating_sub(1) {
                let (a, b) = (l[i], l[i + 1]);
                let now = pair(a, b, ups, pos) + pair(a, b, downs, pos);
                let swapped = pair(b, a, ups, pos) + pair(b, a, downs, pos);
                if swapped < now {
                    l.swap(i, i + 1);
                    pos[a] = i + 1;
                    pos[b] = i;
                    better = true;
                }
            }
        }
        if !better {
            break;
        }
    }
}

// ---- Ports

/// One end of a connector, for spreading ends along a side.
pub(crate) struct End<'a> {
    pub shape: &'a str,
    pub side: Side,
    /// Where the connector heads from here: the other end, or its first bend.
    pub toward: Pt,
}

/// Where along its side each end should meet the box, from 0 to 1: ends sharing a side
/// are spread out, in the order of where they head, so they don't meet in one point or
/// cross each other on the way out. None for an end alone on its side (the middle).
pub(crate) fn spread_ends(ends: &[End]) -> Vec<Option<f32>> {
    let mut groups: HashMap<(&str, Side), Vec<usize>> = HashMap::new();
    for (i, e) in ends.iter().enumerate() {
        groups.entry((e.shape, e.side)).or_default().push(i);
    }
    let mut out = vec![None; ends.len()];
    for ((_, side), mut idx) in groups {
        if idx.len() < 2 {
            continue;
        }
        let key = |i: usize| match side {
            Side::Top | Side::Bottom => ends[i].toward.0,
            Side::Left | Side::Right => ends[i].toward.1,
        };
        idx.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
        let n = idx.len() as f32;
        for (k, &i) in idx.iter().enumerate() {
            out[i] = Some(0.15 + 0.7 * (k as f32 + 1.) / (n + 1.));
        }
    }
    out
}

// ---- Parallel connectors

/// Room between connectors that would otherwise run along the same line.
const LANE: f32 = 10.;

/// Pull apart the middle segments of elbow connectors that run along the same line, so
/// each can be followed on its own.
pub(crate) fn separate_lanes(wires: &mut [(String, Wire)]) {
    for vertical in [true, false] {
        // (wire, segment, line, from, to, where the wire ends up across)
        let mut segs: Vec<(usize, usize, f32, f32, f32, f32)> = Vec::new();
        for (wi, (_, w)) in wires.iter().enumerate() {
            let p = &w.points;
            if w.curved || p.len() < 4 {
                continue;
            }
            for s in 1..p.len() - 2 {
                let (a, b) = (p[s], p[s + 1]);
                let (line, lo, hi, other) = if vertical {
                    if (a.0 - b.0).abs() > 0.5 {
                        continue;
                    }
                    (a.0, a.1.min(b.1), a.1.max(b.1), p[p.len() - 1].0)
                } else {
                    if (a.1 - b.1).abs() > 0.5 {
                        continue;
                    }
                    (a.1, a.0.min(b.0), a.0.max(b.0), p[p.len() - 1].1)
                };
                if hi - lo > 1. {
                    segs.push((wi, s, line, lo, hi, other));
                }
            }
        }
        segs.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.3.total_cmp(&b.3)));
        let mut i = 0;
        while i < segs.len() {
            // Segments on (almost) the same line that overlap along it.
            let mut j = i + 1;
            let mut reach = segs[i].4;
            while j < segs.len() && (segs[j].2 - segs[i].2).abs() < 3. && segs[j].3 < reach - 1. {
                reach = reach.max(segs[j].4);
                j += 1;
            }
            let mut group: Vec<_> = segs[i..j].to_vec();
            group.dedup_by_key(|g| g.0);
            if group.len() > 1 {
                group.sort_by(|a, b| a.5.total_cmp(&b.5));
                let n = group.len() as f32;
                for (k, g) in group.iter().enumerate() {
                    let off = (k as f32 - (n - 1.) / 2.) * LANE;
                    let pts = &mut wires[g.0].1.points;
                    for q in [g.1, g.1 + 1] {
                        if vertical {
                            pts[q].0 += off;
                        } else {
                            pts[q].1 += off;
                        }
                    }
                }
            }
            i = j;
        }
    }
}

// ---- Score

/// How tangled a diagram looks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Score {
    /// Places where two connectors cross.
    pub crossings: usize,
    /// Connectors drawn through a box that isn't one of their ends.
    pub through: usize,
    /// Boxes drawn over each other.
    pub overlaps: usize,
    pub bends: usize,
    pub length: f32,
}

impl Score {
    /// One number to compare layouts by: crossings and boxes in the way count most.
    pub(crate) fn cost(&self) -> f32 {
        self.crossings as f32 * 10.
            + self.through as f32 * 14.
            + self.overlaps as f32 * 30.
            + self.bends as f32
            + self.length / 400.
    }

    pub(crate) fn is_clean(&self) -> bool {
        self.crossings == 0 && self.through == 0 && self.overlaps == 0
    }

    /// What's wrong, for people: "3 crossings, 1 overlap".
    pub(crate) fn describe(&self) -> String {
        let mut parts = Vec::new();
        let mut add = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        add(self.crossings, "crossing", "crossings");
        add(self.through, "line through a shape", "lines through shapes");
        add(self.overlaps, "overlap", "overlaps");
        if parts.is_empty() {
            "no crossings or overlaps".into()
        } else {
            parts.join(", ")
        }
    }
}

/// Score a diagram from its boxes and routed connectors. `solid` are the boxes a
/// connector shouldn't cross (not boundaries); each wire names the boxes it joins.
pub(crate) fn score(solid: &[(&str, Rect)], wires: &[(&Wire, &str, &str)]) -> Score {
    let mut s = Score::default();
    let lines: Vec<Vec<Pt>> = wires.iter().map(|(w, _, _)| w.polyline()).collect();
    for (k, l) in lines.iter().enumerate() {
        if !wires[k].0.curved {
            s.bends += l.len().saturating_sub(2);
        }
        s.length += l
            .windows(2)
            .map(|p| super::geom::dist(p[0], p[1]))
            .sum::<f32>();
    }
    // Crossings between different connectors, not counting ones that just share a box.
    for i in 0..lines.len() {
        let bi = Rect::bounds(lines[i].iter().map(|p| Rect::new(p.0, p.1, 0., 0.)));
        for j in i + 1..lines.len() {
            let bj = Rect::bounds(lines[j].iter().map(|p| Rect::new(p.0, p.1, 0., 0.)));
            if let (Some(a), Some(b)) = (bi, bj) {
                if !a.inflate(1.).intersects(&b.inflate(1.)) {
                    continue;
                }
            }
            for p in lines[i].windows(2) {
                for q in lines[j].windows(2) {
                    if proper_cross(p[0], p[1], q[0], q[1]) {
                        s.crossings += 1;
                    }
                }
            }
        }
    }
    for (k, l) in lines.iter().enumerate() {
        let (_, a, b) = wires[k];
        for (id, r) in solid {
            if *id == a || *id == b {
                continue;
            }
            let inner = r.inflate(-2.);
            if l.windows(2).any(|p| seg_through(p[0], p[1], &inner)) {
                s.through += 1;
            }
        }
    }
    for i in 0..solid.len() {
        for j in i + 1..solid.len() {
            let (a, b) = (solid[i].1, solid[j].1);
            if a.inflate(-1.).intersects(&b.inflate(-1.))
                && !a.contains_rect(&b)
                && !b.contains_rect(&a)
            {
                s.overlaps += 1;
            }
        }
    }
    s
}

/// Whether two segments cross somewhere other than near their ends.
fn proper_cross(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let orient = |p: Pt, q: Pt, r: Pt| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (d1, d2) = (orient(c, d, a), orient(c, d, b));
    let (d3, d4) = (orient(a, b, c), orient(a, b, d));
    let eps = 1e-3;
    if !((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps)) {
        return false;
    }
    if !((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps)) {
        return false;
    }
    // Lines that meet at a shared end (fanning out of one box) don't count.
    let near = |p: Pt, q: Pt| (p.0 - q.0).abs() < 2. && (p.1 - q.1).abs() < 2.;
    !(near(a, c) || near(a, d) || near(b, c) || near(b, d))
}

/// Whether a segment passes through the inside of `r`.
fn seg_through(a: Pt, b: Pt, r: &Rect) -> bool {
    if r.w <= 0. || r.h <= 0. {
        return false;
    }
    if r.contains(a) || r.contains(b) {
        return true;
    }
    let corners = [
        (r.x, r.y),
        (r.right(), r.y),
        (r.right(), r.bottom()),
        (r.x, r.bottom()),
    ];
    (0..4).any(|i| {
        let (c, d) = (corners[i], corners[(i + 1) % 4]);
        let orient = |p: Pt, q: Pt, s: Pt| (q.0 - p.0) * (s.1 - p.1) - (q.1 - p.1) * (s.0 - p.0);
        let (d1, d2) = (orient(c, d, a), orient(c, d, b));
        let (d3, d4) = (orient(a, b, c), orient(a, b, d));
        ((d1 > 0. && d2 < 0.) || (d1 < 0. && d2 > 0.))
            && ((d3 > 0. && d4 < 0.) || (d3 < 0. && d4 > 0.))
    })
}

// ---- Tidy in place

/// Line up boxes whose centres almost line up, across and down, so the connectors
/// between them run straight. Returns new centres.
pub(crate) fn align_near(centres: &[Pt], within: f32) -> Vec<Pt> {
    let mut out = centres.to_vec();
    for axis in 0..2 {
        let get = |p: &Pt| if axis == 0 { p.0 } else { p.1 };
        let mut idx: Vec<usize> = (0..out.len()).collect();
        idx.sort_by(|&a, &b| get(&out[a]).total_cmp(&get(&out[b])));
        let mut i = 0;
        while i < idx.len() {
            let mut j = i + 1;
            while j < idx.len() && get(&out[idx[j]]) - get(&out[idx[j - 1]]) <= within {
                j += 1;
            }
            if j - i > 1 {
                let mut vals: Vec<f32> = idx[i..j].iter().map(|&k| get(&out[k])).collect();
                vals.sort_by(f32::total_cmp);
                let mid = vals[vals.len() / 2];
                for &k in &idx[i..j] {
                    if axis == 0 {
                        out[k].0 = mid;
                    } else {
                        out[k].1 = mid;
                    }
                }
            }
            i = j;
        }
    }
    out
}

/// Push boxes apart until none overlap, keeping at least `gap` between them. Each step
/// moves a pair the short way out; `fixed` boxes don't move.
pub(crate) fn push_apart(rects: &mut [Rect], fixed: &[bool], gap: f32) {
    for _ in 0..80 {
        let mut moved = false;
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                if fixed[i] && fixed[j] {
                    continue;
                }
                let (a, b) = (rects[i].inflate(gap / 2.), rects[j].inflate(gap / 2.));
                if !a.intersects(&b) {
                    continue;
                }
                let dx = (a.right().min(b.right()) - a.x.max(b.x)).max(0.);
                let dy = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.);
                let (ca, cb) = (a.center(), b.center());
                let (mx, my) = if dx < dy {
                    (if ca.0 <= cb.0 { -dx } else { dx }, 0.)
                } else {
                    (0., if ca.1 <= cb.1 { -dy } else { dy })
                };
                let (si, sj) = match (fixed[i], fixed[j]) {
                    (true, _) => (0., 1.),
                    (_, true) => (1., 0.),
                    _ => (0.5, 0.5),
                };
                rects[i].x += mx * si;
                rects[i].y += my * si;
                rects[j].x -= mx * sj;
                rects[j].y -= my * sj;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// Straight-line crossings among connectors between box centres: a quick stand-in for
/// routed crossings when trying many arrangements.
pub(crate) fn quick_crossings(centres: &HashMap<&str, Pt>, links: &[(&str, &str)]) -> usize {
    let segs: Vec<(Pt, Pt)> = links
        .iter()
        .filter_map(|(a, b)| Some((*centres.get(a)?, *centres.get(b)?)))
        .collect();
    let mut c = 0;
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            if proper_cross(segs[i].0, segs[i].1, segs[j].0, segs[j].1) {
                c += 1;
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn links(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn a_crossed_pair_is_untangled() {
        // Two callers each calling the callee under the other: drawn in the given order
        // they cross; the layout must put each callee under its caller.
        let ids = ids(&["a", "b", "y", "x"]);
        let l = links(&[("a", "x"), ("b", "y")]);
        let pos = layered(&ids, &HashMap::new(), &l, (0., 0.), false);
        assert!(pos["a"].1 < pos["x"].1);
        let a_left = pos["a"].0 < pos["b"].0;
        let x_left = pos["x"].0 < pos["y"].0;
        // Separate groups side by side: each callee sits in its caller's column.
        assert_eq!(a_left, x_left);
    }

    #[test]
    fn layering_counts_crossings_away() {
        // K(2,2) plus a skip link: a classic tangle for a naive order.
        let ids = ids(&["web", "app", "api", "worker", "db", "queue"]);
        let l = links(&[
            ("web", "api"),
            ("app", "api"),
            ("web", "worker"),
            ("api", "db"),
            ("worker", "queue"),
            ("app", "db"),
        ]);
        let pos = layered(&ids, &HashMap::new(), &l, (0., 0.), false);
        let sizes = (180., 90.);
        let centres: HashMap<&str, Pt> = pos
            .iter()
            .map(|(k, p)| (k.as_str(), (p.0 + sizes.0 / 2., p.1 + sizes.1 / 2.)))
            .collect();
        let ls: Vec<(&str, &str)> = l.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert!(quick_crossings(&centres, &ls) <= 1);
        // No two boxes overlap.
        let rects: Vec<Rect> = pos
            .values()
            .map(|p| Rect::new(p.0, p.1, 180., 90.))
            .collect();
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!rects[i].intersects(&rects[j]));
            }
        }
    }

    #[test]
    fn long_links_leave_a_channel() {
        // top calls mid, mid calls bottom, and top calls bottom directly: the long link
        // gets its own room, so mid doesn't sit in its way.
        let ids = ids(&["top", "mid", "bottom"]);
        let l = links(&[("top", "mid"), ("mid", "bottom"), ("top", "bottom")]);
        let pos = layered(&ids, &HashMap::new(), &l, (0., 0.), false);
        assert!(pos["top"].1 < pos["mid"].1 && pos["mid"].1 < pos["bottom"].1);
        let r = |id: &str| Rect::new(pos[id].0, pos[id].1, 180., 90.);
        let mid = r("mid");
        // Routed round mid, the long link stays clear of it with few bends.
        let w = super::super::geom::route_with(
            &r("top"),
            &r("bottom"),
            super::super::model::Route::Elbow,
            &super::super::geom::Way {
                obstacles: &[mid],
                ..Default::default()
            },
        );
        assert!(!w
            .points
            .windows(2)
            .any(|p| seg_through(p[0], p[1], &mid.inflate(-2.))));
        assert!(w.points.len() <= 6, "{:?}", w.points);
    }

    #[test]
    fn lone_boxes_go_in_a_grid_after_the_rest() {
        let ids = ids(&["a", "b", "c", "d", "e"]);
        let l = links(&[("a", "b")]);
        let pos = layered(&ids, &HashMap::new(), &l, (0., 0.), false);
        let right_of_group = pos["a"].0.max(pos["b"].0) + 180.;
        for id in ["c", "d", "e"] {
            assert!(pos[id].0 >= right_of_group);
        }
    }

    #[test]
    fn ends_on_one_side_are_spread_in_order() {
        let ends = [
            End {
                shape: "api",
                side: Side::Bottom,
                toward: (500., 400.),
            },
            End {
                shape: "api",
                side: Side::Bottom,
                toward: (100., 400.),
            },
            End {
                shape: "db",
                side: Side::Top,
                toward: (0., 0.),
            },
        ];
        let t = spread_ends(&ends);
        assert_eq!(t[2], None);
        let (right, left) = (t[0].unwrap(), t[1].unwrap());
        assert!(left < 0.5 && right > 0.5);
    }

    #[test]
    fn overlapping_lanes_are_pulled_apart() {
        let w = |pts: Vec<Pt>| Wire {
            points: pts,
            curved: false,
        };
        let mut wires = vec![
            (
                "a".to_string(),
                w(vec![
                    (0., 0.),
                    (0., 50.),
                    (100., 50.),
                    (100., 200.),
                    (150., 200.),
                ]),
            ),
            (
                "b".to_string(),
                w(vec![
                    (20., 0.),
                    (20., 60.),
                    (100., 60.),
                    (100., 180.),
                    (60., 180.),
                ]),
            ),
        ];
        separate_lanes(&mut wires);
        let xa = wires[0].1.points[2].0;
        let xb = wires[1].1.points[2].0;
        assert!((xa - xb).abs() >= LANE - 0.1);
        // Still square: the moved segment stays vertical.
        assert_eq!(wires[0].1.points[2].0, wires[0].1.points[3].0);
    }

    #[test]
    fn the_score_sees_crossings_and_shapes_in_the_way() {
        let w1 = Wire {
            points: vec![(0., 50.), (200., 50.)],
            curved: false,
        };
        let w2 = Wire {
            points: vec![(100., 0.), (100., 100.)],
            curved: false,
        };
        let solid = [("box", Rect::new(140., 30., 30., 40.))];
        let s = score(&solid, &[(&w1, "a", "b"), (&w2, "c", "d")]);
        assert_eq!(s.crossings, 1);
        assert_eq!(s.through, 1);
        assert!(!s.is_clean());
        assert_eq!(s.describe(), "1 crossing, 1 line through a shape");
    }

    #[test]
    fn near_lines_snap_together_and_overlaps_part() {
        let c = align_near(&[(100., 0.), (108., 300.), (400., 4.)], 16.);
        assert_eq!(c[0].0, c[1].0);
        assert_eq!(c[0].1, c[2].1);
        let mut r = vec![
            Rect::new(0., 0., 100., 100.),
            Rect::new(50., 10., 100., 100.),
        ];
        push_apart(&mut r, &[false, false], 20.);
        assert!(!r[0].inflate(9.).intersects(&r[1].inflate(9.)));
    }
}
