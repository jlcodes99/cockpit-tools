// Scrollbar controls live outside the React tree so lists, grids and textareas
// keep their original DOM. Treat those controls as part of their scroll owner.
const owners = new WeakMap<Node, Node>();

export function registerScrollbarOwner(controls: Node, owner: Node): void {
  owners.set(controls, owner);
}

export function containsWithScrollbar(
  container: Node | null | undefined,
  target: Node | null | undefined,
): boolean {
  if (!container || !target) return false;
  if (container.contains(target)) return true;
  for (let node: Node | null = target; node; node = node.parentNode) {
    const owner = owners.get(node);
    if (owner) return container.contains(owner);
  }
  return false;
}
