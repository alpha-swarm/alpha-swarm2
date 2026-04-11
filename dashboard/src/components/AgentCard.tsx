import { useState, useEffect } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolCallList } from "@/components/ToolCallList";
import { AttemptRow } from "@/components/AttemptRow";
import { resources } from "@/lib/mcp";
import type { AgentRun } from "@/types/swarm";

const POLL_INTERVAL_MS = 5_000;
const TASK_PREVIEW_LENGTH = 100;

function useAgentDetail(expanded: boolean, runId: string | null) {
  const [detail, setDetail] = useState<AgentRun | null>(null);

  useEffect(() => {
    if (!expanded || !runId) return;
    const fetchDetail = async () => {
      try {
        const data = await resources.runDetail(runId);
        if (data[0]) setDetail(data[0]);
      } catch { /* ignore */ }
    };
    fetchDetail();
    const interval = setInterval(fetchDetail, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [expanded, runId]);

  return detail;
}

function AgentCardHeader({ run, expanded }: { run: AgentRun; expanded: boolean }) {
  return (
    <div className="flex items-center gap-3">
      <StatusBadge status={run.status} />
      <CardTitle className="text-sm font-medium flex-1 truncate">
        {run.task_description.slice(0, TASK_PREVIEW_LENGTH)}
      </CardTitle>
      <span className="text-xs text-muted-foreground whitespace-nowrap">
        {run.model_used} | {run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "..."}
      </span>
      <span className="text-muted-foreground text-xs">{expanded ? "^" : "v"}</span>
    </div>
  );
}

function PhaseTimingTags({ timings }: { timings: Record<string, number> }) {
  return (
    <div className="flex gap-2 text-xs">
      {Object.entries(timings).map(([k, v]) => (
        <span key={k} className="bg-muted px-2 py-0.5 rounded">
          {k.replace("_ms", "")}: {((v as number) / 1000).toFixed(1)}s
        </span>
      ))}
    </div>
  );
}

function AgentCardExpanded({ run }: { run: AgentRun }) {
  return (
    <CardContent className="space-y-3">
      {run.error_message && (
        <div className="text-xs text-destructive bg-destructive/10 rounded p-2">
          {run.error_message}
        </div>
      )}
      {run.phase_timings && <PhaseTimingTags timings={run.phase_timings as unknown as Record<string, number>} />}
      {run.diff && (
        <div>
          <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">Diff</div>
          <pre className="bg-muted/50 rounded p-2 text-[11px] font-mono whitespace-pre-wrap max-h-48 overflow-auto">
            {run.diff}
          </pre>
        </div>
      )}
      {run.attempts.length > 0 && (
        <div>
          <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">
            Attempts ({run.attempts.length})
          </div>
          {run.attempts.map((a, i) => <AttemptRow key={i} attempt={a} compact />)}
        </div>
      )}
      {run.tool_calls.length > 0 && (
        <div>
          <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">
            Tool Calls ({run.tool_calls.length})
          </div>
          <ToolCallList calls={run.tool_calls} />
        </div>
      )}
    </CardContent>
  );
}

export function AgentCard({ run }: { run: AgentRun }) {
  const [expanded, setExpanded] = useState(false);
  const detail = useAgentDetail(expanded, run.id);
  const fullRun = detail ?? run;

  return (
    <Card className={run.status === "running" ? "border-yellow-500/30" : ""}>
      <CardHeader className="pb-2 cursor-pointer" onClick={() => setExpanded(!expanded)}>
        <AgentCardHeader run={fullRun} expanded={expanded} />
        {fullRun.progress_message && (
          <p className="text-xs text-muted-foreground mt-1 font-mono truncate">
            {fullRun.progress_message}
          </p>
        )}
      </CardHeader>
      {expanded && <AgentCardExpanded run={fullRun} />}
    </Card>
  );
}
