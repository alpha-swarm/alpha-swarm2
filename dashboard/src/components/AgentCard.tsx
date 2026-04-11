import { useState, useEffect } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolCallList } from "@/components/ToolCallList";
import { resources } from "@/lib/mcp";
import type { AgentRun } from "@/types/swarm";

const POLL_INTERVAL_MS = 5_000;
const TASK_PREVIEW_LENGTH = 100;

interface AgentCardProps {
  run: AgentRun;
  depth?: number;
  maxDepth?: number;
}

export function AgentCard({ run, depth = 0, maxDepth = 3 }: AgentCardProps) {
  const [expanded, setExpanded] = useState(run.status === "running");
  const [detail, setDetail] = useState<AgentRun | null>(null);
  const [subRuns, setSubRuns] = useState<AgentRun[]>([]);

  useEffect(() => {
    if (!expanded || !run.id) return;
    const fetch = async () => {
      try {
        const [d, s] = await Promise.all([
          resources.runDetail(run.id!),
          depth < maxDepth ? resources.subRuns(run.id!) : Promise.resolve([]),
        ]);
        if (d[0]) setDetail(d[0]);
        setSubRuns(s);
      } catch { /* ignore */ }
    };
    fetch();
    const interval = setInterval(fetch, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [expanded, run.id, depth, maxDepth]);

  const fullRun = detail ?? run;
  const opacity = Math.max(0.5, 1 - depth * 0.15);

  return (
    <div style={{ marginLeft: `${depth * 16}px`, opacity }}>
      <Card className={run.status === "running" ? "border-yellow-500/30" : ""}>
        <CardHeader className="pb-2 cursor-pointer" onClick={() => setExpanded(!expanded)}>
          <div className="flex items-center gap-3">
            <StatusBadge status={fullRun.status} />
            <CardTitle className="text-sm font-medium flex-1 truncate">
              {fullRun.task_description.slice(0, TASK_PREVIEW_LENGTH)}
            </CardTitle>
            {depth > 0 && (
              <Badge variant="outline" className="text-[9px]">depth {depth}/{maxDepth}</Badge>
            )}
            <span className="text-xs text-muted-foreground whitespace-nowrap">
              {fullRun.model_used} | {fullRun.duration_ms > 0 ? `${(fullRun.duration_ms / 1000).toFixed(0)}s` : "..."}
            </span>
            <span className="text-muted-foreground text-xs">{expanded ? "\u25B2" : "\u25BC"}</span>
          </div>
          {fullRun.progress_message && (
            <p className="text-xs text-muted-foreground mt-1 font-mono truncate">
              {fullRun.progress_message}
            </p>
          )}
        </CardHeader>

        {expanded && (
          <CardContent className="space-y-3">
            {fullRun.error_message && (
              <div className="text-xs text-destructive bg-destructive/10 rounded p-2">
                {fullRun.error_message}
              </div>
            )}

            {fullRun.phase_timings && (
              <div className="flex gap-2 flex-wrap text-xs">
                {Object.entries(fullRun.phase_timings).map(([k, v]) => (
                  <Badge key={k} variant="outline" className="text-[10px]">
                    {k.replace("_ms", "")}: {((v as number) / 1000).toFixed(1)}s
                  </Badge>
                ))}
              </div>
            )}

            {(fullRun.tool_calls?.length ?? 0) > 0 && (
              <div>
                <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">
                  Tool Calls ({fullRun.tool_calls.length})
                </div>
                <ToolCallList calls={fullRun.tool_calls} />
              </div>
            )}

            {fullRun.diff && (
              <details>
                <summary className="text-[10px] text-muted-foreground cursor-pointer font-semibold uppercase">Diff</summary>
                <pre className="bg-muted/50 rounded p-2 text-[11px] font-mono whitespace-pre-wrap max-h-48 overflow-auto mt-1">
                  {fullRun.diff}
                </pre>
              </details>
            )}

            {/* Recursive sub-agents */}
            {subRuns.length > 0 && (
              <div className="mt-2">
                <div className="text-[10px] text-muted-foreground mb-2 font-semibold uppercase">
                  Sub-agents ({subRuns.length})
                </div>
                <div className="space-y-1.5 border-l-2 border-muted pl-1">
                  {subRuns.map((sub) => (
                    <AgentCard key={sub.id} run={sub} depth={depth + 1} maxDepth={maxDepth} />
                  ))}
                </div>
              </div>
            )}
          </CardContent>
        )}
      </Card>
    </div>
  );
}
