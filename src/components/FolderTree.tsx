import { useMemo, useState } from "react";
import { useLibrary } from "../stores/library";
import { useApp } from "../stores/app";

interface Node {
  name: string;
  path: string;
  count: number; // pliki bezpośrednio w folderze
  children: Node[];
}

function buildTree(folders: [string, number][]): Node {
  const root: Node = { name: "", path: "", count: 0, children: [] };
  const byPath = new Map<string, Node>([["", root]]);

  const ensure = (path: string): Node => {
    const existing = byPath.get(path);
    if (existing) return existing;
    const idx = path.lastIndexOf("/");
    const parent = ensure(idx === -1 ? "" : path.slice(0, idx));
    const node: Node = {
      name: idx === -1 ? path : path.slice(idx + 1),
      path,
      count: 0,
      children: [],
    };
    parent.children.push(node);
    byPath.set(path, node);
    return node;
  };

  for (const [path, count] of folders) {
    ensure(path).count = count;
  }
  const sort = (n: Node) => {
    n.children.sort((a, b) => a.name.localeCompare(b.name, "pl"));
    n.children.forEach(sort);
  };
  sort(root);
  return root;
}

function TreeNode({ node, depth }: { node: Node; depth: number }) {
  const [open, setOpen] = useState(depth < 1);
  const parent = useLibrary((s) => s.q.parent);
  const setFolder = useLibrary((s) => s.setFolder);
  const setView = useApp((s) => s.setView);
  const active = parent === node.path;

  return (
    <div>
      <div
        className={`flex cursor-pointer items-center gap-1 rounded-md py-1 pr-2 text-[13px] transition-colors duration-100 ${
          active ? "bg-accent/15 text-ink" : "text-ink-dim hover:bg-raised hover:text-ink"
        }`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() => {
          setFolder(node.path);
          setView("folders");
        }}
      >
        <span
          className={`w-3 shrink-0 text-center text-[9px] text-ink-faint ${
            node.children.length ? "" : "invisible"
          }`}
          onClick={(e) => {
            e.stopPropagation();
            setOpen(!open);
          }}
        >
          {open ? "▼" : "▶"}
        </span>
        <span className="truncate">{node.name || "Cała biblioteka"}</span>
        {node.count > 0 && (
          <span className="ml-auto shrink-0 text-[11px] text-ink-faint">{node.count}</span>
        )}
      </div>
      {open &&
        node.children.map((child) => (
          <TreeNode key={child.path} node={child} depth={depth + 1} />
        ))}
    </div>
  );
}

export function FolderTree() {
  const folders = useLibrary((s) => s.folders);
  const tree = useMemo(() => buildTree(folders), [folders]);
  return <TreeNode node={tree} depth={0} />;
}
