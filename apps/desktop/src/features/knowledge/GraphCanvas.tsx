import React, { useEffect, useRef, useState } from "react";
import Graph from "graphology";
import forceAtlas2 from "graphology-layout-forceatlas2";
import Sigma from "sigma";
import type { EdgeDisplayData, NodeDisplayData } from "sigma/types";
import { Maximize2, ZoomIn, ZoomOut } from "lucide-react";
import type { GraphEdge, GraphNode } from "@symphony/shared-types";
import { Button, Hint } from "@symphony/ui";
import type { Mark } from "./knowledgeLogic.js";
import { readPalette, type GraphPalette } from "./graphPalette.js";

export interface GraphCanvasLabels {
  laying: string;
  /** Shown when the graph could not be built or the renderer could not start (for example no WebGL). */
  failed: string;
  canvas: string;
  zoomIn: string;
  zoomOut: string;
  fit: string;
}

export interface GraphCanvasProps {
  nodes: ReadonlyArray<GraphNode>;
  edges: ReadonlyArray<GraphEdge>;
  /** Nodes a selected commit marks; when non-empty everything else is dimmed. */
  marks: ReadonlyMap<string, Mark>;
  selectedId: string | null;
  onSelect(id: string | null): void;
  /** Double click: drill into the node. */
  onOpen?(id: string): void;
  labels: GraphCanvasLabels;
}

/** Stable pseudo-random position in the unit disc from a node id, so refreshes do not reshuffle. */
function seedPosition(id: string): { x: number; y: number } {
  let h = 2166136261;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  const angle = ((h >>> 0) % 36000) / 36000 * Math.PI * 2;
  const radius = Math.sqrt((((h >>> 8) >>> 0) % 1000) / 1000);
  return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius };
}

function nodeSize(node: GraphNode): number {
  // Screen pixels: modules are the largest nodes, symbols the smallest; both grow with connectivity.
  const base = node.kind === "module" || node.kind === "package" ? 5 + Math.sqrt(node.members) * 0.9 : 4 + Math.sqrt(node.degree) * 0.8;
  return Math.min(14, Math.max(4, base));
}

/** Layout effort: enough iterations to settle, fewer for big graphs so the UI stays responsive. */
function layoutIterations(nodeCount: number): number {
  if (nodeCount <= 150) return 400;
  if (nodeCount <= 600) return 220;
  return 120;
}

function buildGraph(nodes: ReadonlyArray<GraphNode>, edges: ReadonlyArray<GraphEdge>, palette: GraphPalette): Graph {
  const graph = new Graph({ type: "directed", multi: false, allowSelfLoops: false });
  for (const node of nodes) {
    const { x, y } = seedPosition(node.id);
    graph.addNode(node.id, {
      x,
      y,
      size: nodeSize(node),
      label: node.label,
      color: palette.byKind[node.kind],
      kind: node.kind
    });
  }
  // Extracted links first, so when two relations connect the same pair the firmer one is drawn.
  const ordered = [...edges].sort((a, b) => Number(b.confidence === "extracted") - Number(a.confidence === "extracted"));
  for (const edge of ordered) {
    if (edge.source === edge.target || !graph.hasNode(edge.source) || !graph.hasNode(edge.target)) continue;
    if (graph.hasDirectedEdge(edge.source, edge.target)) continue;
    graph.addDirectedEdge(edge.source, edge.target, {
      size: Math.min(4, 1 + Math.log2(1 + edge.weight) * 0.6),
      color: edge.confidence === "extracted" ? palette.edge : palette.edgeWeak,
      confidence: edge.confidence
    });
  }
  return graph;
}

/**
 * Pull nodes that ended up far from the rest back towards it. The renderer fits the whole graph
 * to the view, so a single stray node would otherwise shrink everything else to a speck.
 */
function clampOutliers(graph: Graph): void {
  const points: { id: string; x: number; y: number }[] = [];
  graph.forEachNode((id, attrs) => points.push({ id, x: attrs.x as number, y: attrs.y as number }));
  if (points.length < 5) return;
  const cx = points.reduce((s, p) => s + p.x, 0) / points.length;
  const cy = points.reduce((s, p) => s + p.y, 0) / points.length;
  const distances = points.map((p) => Math.hypot(p.x - cx, p.y - cy)).sort((a, b) => a - b);
  const limit = distances[Math.floor(distances.length * 0.9)]! * 2;
  if (!(limit > 0)) return;
  for (const p of points) {
    const d = Math.hypot(p.x - cx, p.y - cy);
    if (d > limit) {
      graph.setNodeAttribute(p.id, "x", cx + ((p.x - cx) / d) * limit);
      graph.setNodeAttribute(p.id, "y", cy + ((p.y - cy) / d) * limit);
    }
  }
}

/** Re-read the palette when the document theme changes. */
function useThemeVersion(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const observer = new MutationObserver(() => setVersion((v) => v + 1));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-density", "class"] });
    return () => observer.disconnect();
  }, []);
  return version;
}

/**
 * The interactive graph (Sigma.js on a Graphology graph, laid out with ForceAtlas2). The canvas is
 * a visual aid only: every node is also reachable through the node list and the explorer, which are
 * keyboard accessible.
 */
const GraphCanvas: React.FC<GraphCanvasProps> = ({ nodes, edges, marks, selectedId, onSelect, onOpen, labels }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const sigmaRef = useRef<Sigma | null>(null);
  const [laying, setLaying] = useState(true);
  const [failed, setFailed] = useState(false);
  const themeVersion = useThemeVersion();
  const callbacks = useRef({ onSelect, onOpen });
  callbacks.current = { onSelect, onOpen };
  const view = useRef({ marks, selectedId, palette: null as GraphPalette | null });
  view.current.marks = marks;
  view.current.selectedId = selectedId;

  // Build the graph, lay it out and (re)create the renderer when the data or the theme changes.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    let cancelled = false;
    setLaying(true);
    setFailed(false);
    const draw = () => {
      const palette = readPalette(container);
      view.current.palette = palette;
      const graph = buildGraph(nodes, edges, palette);
      if (graph.order > 1) {
        forceAtlas2.assign(graph, {
          iterations: layoutIterations(graph.order),
          // Strong gravity keeps unconnected nodes near the others instead of drifting away.
          settings: { ...forceAtlas2.inferSettings(graph), slowDown: 2, barnesHutOptimize: graph.order > 400, strongGravityMode: true, gravity: 0.8 }
        });
        clampOutliers(graph);
      }
      const renderer = new Sigma(graph, container, {
        settings: {
          renderEdgeLabels: false,
          // Node sizes are screen pixels and do not grow with the zoom, so a dense graph stays legible.
          itemSizesReference: "screen",
          zoomToSizeRatioFunction: () => 1,
          // Every node may carry a label; the renderer's grid hides the ones that would overlap.
          labelRenderedSizeThreshold: 0,
          labelDensity: 0.7,
          minCameraRatio: 0.05,
          maxCameraRatio: 10
        },
        nodeReducer: (node, data): Partial<NodeDisplayData> => {
          const { marks: active, selectedId: selected, palette: p } = view.current;
          // Labels and their hover backdrop follow the theme instead of the renderer's defaults.
          const result: Partial<NodeDisplayData> = p ? { labelColor: p.label, backdropColor: p.surface, backdropShadowColor: p.dim } : {};
          if (active.size > 0 && p) {
            const mark = active.get(node);
            if (mark) {
              result.color = mark.emphasis === "changed" ? p.changed : p.affected;
              result.zIndex = mark.emphasis === "changed" ? 2 : 1;
              result.labelVisibility = "visible";
              if (mark.emphasis === "changed") result.size = data.size * 1.35;
            } else {
              result.color = p.dim;
              result.label = null;
              result.labelVisibility = "hidden";
            }
          }
          if (node === selected) {
            result.highlighted = true;
            result.labelVisibility = "visible";
            result.zIndex = 3;
          }
          return result;
        },
        edgeReducer: (edge): Partial<EdgeDisplayData> => {
          const { marks: active, palette: p } = view.current;
          if (active.size === 0 || !p) return {};
          const [source, target] = graph.extremities(edge);
          const both = active.has(source) && active.has(target);
          const either = active.has(source) || active.has(target);
          return { color: both ? p.affected : either ? p.edge : p.dim, visibility: !either && graph.size > 800 ? "hidden" : "visible" };
        }
      });
      renderer.on("clickNode", ({ node }) => callbacks.current.onSelect(node));
      renderer.on("clickStage", () => callbacks.current.onSelect(null));
      renderer.on("doubleClickNode", (payload) => {
        payload.preventSigmaDefault();
        callbacks.current.onOpen?.(payload.node);
      });
      sigmaRef.current?.kill();
      sigmaRef.current = renderer;
      setLaying(false);
    };
    const timer = window.setTimeout(() => {
      if (cancelled) return;
      try {
        draw();
      } catch (error) {
        // A browser without WebGL, or data the renderer rejects: say so instead of waiting forever.
        console.error("knowledge graph", error);
        setFailed(true);
        setLaying(false);
      }
    }, 0);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      sigmaRef.current?.kill();
      sigmaRef.current = null;
    };
    // The arrays themselves are the dependency: a re-index that changes only edges must redraw, and
    // the query cache hands out the same arrays while the data is unchanged, so an unchanged graph is
    // not laid out again.
  }, [nodes, edges, themeVersion]);

  // Marks and selection only change how nodes are drawn.
  useEffect(() => {
    sigmaRef.current?.refresh();
  }, [marks, selectedId]);

  const zoom = (direction: "in" | "out" | "fit") => {
    const camera = sigmaRef.current?.getCamera();
    if (!camera) return;
    if (direction === "in") void camera.zoomIn({ duration: 200 });
    else if (direction === "out") void camera.zoomOut({ duration: 200 });
    else void camera.reset({ duration: 200 });
  };

  return (
    <div className="relative size-full min-h-64 overflow-hidden rounded-md border border-border bg-surface-sunken">
      <div ref={containerRef} role="img" aria-label={labels.canvas} className="absolute inset-0 text-foreground" />
      {failed ? (
        <div role="alert" className="absolute inset-0 flex items-center justify-center bg-surface-sunken p-4 text-center text-sm text-destructive">
          {labels.failed}
        </div>
      ) : null}
      {laying ? (
        <div role="status" className="absolute inset-0 flex items-center justify-center bg-surface-sunken/70 text-sm text-muted-foreground">
          {labels.laying}
        </div>
      ) : null}
      <div className="absolute end-2 top-2 flex flex-col gap-1">
        <Hint label={labels.zoomIn} asLabel>
          <Button size="sm" variant="secondary" onClick={() => zoom("in")}>
            <ZoomIn className="size-4" />
          </Button>
        </Hint>
        <Hint label={labels.zoomOut} asLabel>
          <Button size="sm" variant="secondary" onClick={() => zoom("out")}>
            <ZoomOut className="size-4" />
          </Button>
        </Hint>
        <Hint label={labels.fit} asLabel>
          <Button size="sm" variant="secondary" onClick={() => zoom("fit")}>
            <Maximize2 className="size-4" />
          </Button>
        </Hint>
      </div>
    </div>
  );
};

export default GraphCanvas;
