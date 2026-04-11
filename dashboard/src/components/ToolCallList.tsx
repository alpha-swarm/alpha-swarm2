import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import type { ToolCallRecord } from "@/types/swarm";

export function ToolCallList({ calls }: { calls: ToolCallRecord[] }) {
  if (calls.length === 0) return null;

  return (
    <div className="space-y-1">
      {calls.map((tc, i) => (
        <ToolCallRow key={i} call={tc} index={i} />
      ))}
    </div>
  );
}

function ToolCallRow({ call, index }: { call: ToolCallRecord; index: number }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      className={`rounded-md border text-xs ${call.is_error ? "border-destructive/30 bg-destructive/5" : "border-border"}`}
    >
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-muted/50 transition-colors"
      >
        <span className="text-muted-foreground w-5">{index + 1}</span>
        <Badge variant={call.is_error ? "destructive" : "secondary"} className="text-[10px] px-1.5 py-0">
          {call.tool}
        </Badge>
        <span className="flex-1 truncate text-muted-foreground">
          {call.params_preview}
        </span>
        <span className="text-muted-foreground whitespace-nowrap">
          {(call.duration_ms / 1000).toFixed(1)}s
        </span>
        <span className="text-muted-foreground">{expanded ? "^" : "v"}</span>
      </button>

      {expanded && (
        <div className="px-3 pb-3 space-y-2 border-t">
          <div className="pt-2">
            <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">Params</div>
            <pre className="bg-muted/50 rounded p-2 whitespace-pre-wrap break-all font-mono">
              {call.params_preview || "(none)"}
            </pre>
          </div>
          <div>
            <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">Result</div>
            <pre className={`rounded p-2 whitespace-pre-wrap break-all font-mono ${call.is_error ? "bg-destructive/10 text-destructive" : "bg-muted/50"}`}>
              {call.result_preview || "(none)"}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
