"use client";

import { NODE_STRIDE, type EntityKindDto, type Frame, type TopologyView } from "@controlhub/web-engine";
import { externalLabel } from "@/renderer/external";

const STATUS_LABEL = ["No data", "Healthy", "Degraded", "Failing"];
const STATUS_CLASS = ["text-muted", "text-accent", "text-warn", "text-error"];

interface Props {
  frame: Frame | null;
  topology: TopologyView | null;
  selectedId: string | null;
  onSelect: (id: string) => void;
}

/** Accessible, non-GPU representation of the map. */
export function NodeTable({ frame, topology, selectedId, onSelect }: Props) {
  const rows: Array<{ id: string; name: string; kind: EntityKindDto; provenance: string; status: number; active: number; requests: number }> = [];
  if (frame && topology) {
    for (let i = 0; i < frame.nodeIndices.length; i++) {
      const node = topology.nodes[frame.nodeIndices[i] ?? -1];
      if (!node) continue;
      const o = i * NODE_STRIDE;
      rows.push({
        id: node.id,
        name: node.name,
        kind: node.kind,
        provenance: node.provenance,
        status: frame.nodeData[o + 3] ?? 0,
        active: frame.nodeData[o + 4] ?? 0,
        requests: frame.nodeData[o + 5] ?? 0,
      });
    }
  }
  rows.sort((a, b) => b.status - a.status || a.name.localeCompare(b.name));

  if (!frame || !topology) return <p className="p-4 text-sm text-muted">Waiting for engine…</p>;
  if (rows.length === 0) return <p className="p-4 text-sm text-muted">No components match the current filters.</p>;

  return (
    <table className="w-full text-left text-sm">
      <caption className="sr-only">Components at the selected time</caption>
      <thead className="table-head">
        <tr>
          <th scope="col" className="px-4 py-2">Name</th>
          <th scope="col" className="px-4 py-2">Kind</th>
          <th scope="col" className="px-4 py-2">Status</th>
          <th scope="col" className="px-4 py-2 text-right">Active</th>
          <th scope="col" className="px-4 py-2 text-right">Requests (window)</th>
          <th scope="col" className="px-4 py-2">Provenance</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id} className={`border-t border-border hover:bg-surface-2 ${r.id === selectedId ? "bg-accent/10" : ""}`}>
            <td className="px-4 py-1.5">
              <button type="button" className="hover:text-accent" onClick={() => onSelect(r.id)}>
                {r.name}
              </button>
            </td>
            <td className="px-4 py-1.5 text-muted">{externalLabel(r.kind) ?? r.kind}</td>
            <td className={`px-4 py-1.5 ${STATUS_CLASS[r.status] ?? ""}`}>{STATUS_LABEL[r.status] ?? "?"}</td>
            <td className="px-4 py-1.5 text-right tabular-nums">{r.active}</td>
            <td className="px-4 py-1.5 text-right tabular-nums">{r.requests}</td>
            <td className="px-4 py-1.5 text-muted">{r.provenance}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
