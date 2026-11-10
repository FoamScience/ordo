// One hunk's chain of dependencies: what it stands on to its left, what stands on it to its right.

import dagre from "@dagrejs/dagre";
import type { Item } from "./types.ts";

export interface Point {
  x: number;
  y: number;
}

export interface MapEdge {
  /** the hunk that is needed */
  from: number;
  /** the hunk that needs it */
  to: number;
  points: Point[];
  /** marked reviewed while what it needs is not */
  outOfOrder: boolean;
}

export interface DepMap {
  /** each hunk's centre */
  nodes: Map<number, Point>;
  edges: MapEdge[];
  width: number;
  height: number;
}

/** Pairs [needed, needer] among the hunks in `view`, each once. */
export function needPairs(items: Item[], view: number[]): [number, number][] {
  const shown = new Set(view);
  const seen = new Set<string>();
  const out: [number, number][] = [];
  for (const i of view) {
    for (const e of items[i].edges) {
      const t = e.target;
      if (!e.needs || t === null || t === i || !shown.has(t) || seen.has(`${t} ${i}`)) continue;
      seen.add(`${t} ${i}`);
      out.push([t, i]);
    }
  }
  return out;
}

/** Everything reachable from `i` along `next`, without `i` itself. */
function reach(next: Map<number, number[]>, i: number): Set<number> {
  const out = new Set<number>();
  const queue = [i];
  while (queue.length) {
    for (const j of next.get(queue.pop()!) ?? []) {
      if (j === i || out.has(j)) continue;
      out.add(j);
      queue.push(j);
    }
  }
  return out;
}

function adjacency(pairs: [number, number][], up: boolean): Map<number, number[]> {
  const out = new Map<number, number[]>();
  for (const [from, to] of pairs) {
    const [a, b] = up ? [to, from] : [from, to];
    out.set(a, [...(out.get(a) ?? []), b]);
  }
  return out;
}

/** What loses its footing if `i` is rejected: what needs it, and what needs those. */
export function cascadeOf(items: Item[], view: number[], i: number): Set<number> {
  return reach(adjacency(needPairs(items, view), false), i);
}

/**
 * The cascade worth showing beside a hunk's direct dependents: all of it when
 * it reaches past them, else nothing, since it would only repeat them.
 */
export function cascadeBeyond(neededBy: (number | null)[], cascade: number[]): number[] {
  const direct = new Set(neededBy);
  return cascade.some((t) => !direct.has(t)) ? cascade : [];
}

/** What `i` stands on: what it needs, and what those need. */
export function groundOf(items: Item[], view: number[], i: number): Set<number> {
  return reach(adjacency(needPairs(items, view), true), i);
}

/**
 * `i`'s chain laid out left to right, each hunk right of what it needs. A
 * layer can hold dozens of hunks, so layers run down the page, not across it.
 */
export function depMap(items: Item[], view: number[], i: number, node: { width: number; height: number }): DepMap {
  const keep = new Set([i, ...groundOf(items, view, i), ...cascadeOf(items, view, i)]);
  const pairs = needPairs(items, view).filter(([from, to]) => keep.has(from) && keep.has(to));
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: node.height * 0.3, ranksep: node.width * 0.35, marginx: 8, marginy: 8 });
  g.setDefaultEdgeLabel(() => ({}));
  // reading order first, so ties in a layer keep the order the review reads in
  for (const k of view) if (keep.has(k)) g.setNode(String(k), { ...node });
  for (const [from, to] of pairs) g.setEdge(String(from), String(to));
  dagre.layout(g);
  const nodes = new Map<number, Point>();
  for (const id of g.nodes()) {
    const n = g.node(id);
    nodes.set(Number(id), { x: n.x!, y: n.y! });
  }
  const edges = pairs.map(([from, to]) => ({
    from,
    to,
    points: g.edge(String(from), String(to)).points ?? [],
    outOfOrder: items[to].reviewed && !items[from].reviewed,
  }));
  const { width = 0, height = 0 } = g.graph();
  return { nodes, edges, width, height };
}

/** An SVG path through dagre's points, its corners rounded off. */
export function smoothPath(ps: Point[]): string {
  if (!ps.length) return "";
  let d = `M ${ps[0].x} ${ps[0].y}`;
  for (let k = 1; k < ps.length - 1; k++) {
    const mid = { x: (ps[k].x + ps[k + 1].x) / 2, y: (ps[k].y + ps[k + 1].y) / 2 };
    d += ` Q ${ps[k].x} ${ps[k].y} ${mid.x} ${mid.y}`;
  }
  const last = ps[ps.length - 1];
  return `${d} L ${last.x} ${last.y}`;
}
