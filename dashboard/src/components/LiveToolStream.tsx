import { Badge } from "@/components/ui/badge";
import type { ToolCallRecord } from "@/types/swarm";

function ToolLine({ tc }: { tc: ToolCallRecord }) {
  const icon = tc.is_error ? "\u2717" : "\u2713";
  const color = tc.is_error ? "text-red-400" : "text-emerald-400";
  const dur = tc.duration_ms > 0 ? `${(tc.duration_ms / 1000).toFixed(1)}s` : "";
  return (
    <div className="flex items-center gap-1.5 py-px font-mono text-[11px] leading-5">
      <span className={`w-3 text-center ${color}`}>{icon}</span>
      <Badge variant="outline" className="text-[9px] px-1 py-0 h-[14px] font-mono border-border/50">{tc.tool}</Badge>
      <span className="truncate flex-1 text-muted-foreground/70">{tc.params_preview}</span>
      {dur && <span className="text-[10px] text-muted-foreground/50 tabular-nums shrink-0">{dur}</span>}
    </div>
  );
}

export function LiveToolStream({ calls }: { calls: ToolCallRecord[] }) {
  if (calls.length === 0) return null;
  return (
    <div className="border-l border-border/30 ml-2 pl-2.5 py-0.5">
      {calls.map((tc, i) => <ToolLine key={i} tc={tc} />)}
    </div>
  );
}
