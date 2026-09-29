import type { Project } from "./api";

export type ProjectNode = { project: Project; depth: number; children: ProjectNode[] };

/**
 * Arranges projects into a tree by parent_id, siblings in the order given.
 * A project whose parent is missing (or which would loop) is treated as top level.
 */
export function buildProjectTree(projects: Project[]): ProjectNode[] {
  const byId = new Map(projects.map((p) => [p.id, p]));
  const kids = new Map<number | null, Project[]>();
  for (const p of projects) {
    let parent = p.parent_id != null && byId.has(p.parent_id) ? p.parent_id : null;
    // Guard against a loop in the data: walk up and give up if we come back round.
    for (let cur = parent, hops = 0; cur != null; cur = byId.get(cur)?.parent_id ?? null) {
      if (cur === p.id || ++hops > projects.length) {
        parent = null;
        break;
      }
    }
    kids.set(parent, [...(kids.get(parent) ?? []), p]);
  }
  const grow = (parent: number | null, depth: number): ProjectNode[] =>
    (kids.get(parent) ?? []).map((project) => ({ project, depth, children: grow(project.id, depth + 1) }));
  return grow(null, 0);
}

/** Depth-first list of the tree, for indented menus and lists. */
export function flattenTree(nodes: ProjectNode[]): ProjectNode[] {
  return nodes.flatMap((n) => [n, ...flattenTree(n.children)]);
}

/** Ids of a project and everything under it. */
export function subtreeIds(node: ProjectNode): number[] {
  return [node.project.id, ...node.children.flatMap(subtreeIds)];
}
