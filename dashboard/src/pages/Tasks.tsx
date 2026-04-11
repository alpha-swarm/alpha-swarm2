import { useState } from "react";
import { Link } from "react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/StatusBadge";
import { useLiveAgents } from "@/hooks/useLiveAgents";
import { useRuns } from "@/hooks/useRuns";
import type { AgentRun } from "@/types/swarm";

function TaskAccordion({ run }: { run: AgentRun }) {
  const [open, setOpen] = useState(run.status === "running");

  return (
    <Card className={run.status === "running" ? "border-yellow-500/30" : ""}>
      <CardHeader
        className="pb-2 cursor-pointer hover:bg-muted/30 transition-colors"
        onClick={() => setOpen(!open)}
      >
        <div className="flex items-center gap-3">
          <StatusBadge status={run.status} />
          <CardTitle className="text-sm font-medium flex-1 truncate">
            {run.task_description.slice(0, 100)}
          </CardTitle>
          <span className="text-xs text-muted-foreground whitespace-nowrap">
            {run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "..."}
          </span>
          <span className="text-muted-foreground text-xs">{open ? "\u25B2" : "\u25BC"}</span>
        </div>
        {run.progress_message && (
          <p className="text-xs text-muted-foreground mt-1 font-mono truncate">
            {run.progress_message}
          </p>
        )}
      </CardHeader>

      {open && (
        <CardContent className="space-y-3 text-xs">
          <div className="grid grid-cols-3 gap-2">
            <div><span className="text-muted-foreground">ID:</span> <span className="font-mono">{run.id}</span></div>
            <div><span className="text-muted-foreground">Model:</span> {run.model_used}</div>
            <div><span className="text-muted-foreground">Agent:</span> {run.agent_id}</div>
          </div>

          {run.error_message && (
            <div className="text-destructive bg-destructive/10 rounded p-2">{run.error_message}</div>
          )}

          {run.phase_timings && (
            <div className="flex gap-2 flex-wrap">
              {Object.entries(run.phase_timings).map(([k, v]) => (
                <Badge key={k} variant="outline" className="text-[10px]">
                  {k.replace("_ms", "")}: {((v as number) / 1000).toFixed(1)}s
                </Badge>
              ))}
            </div>
          )}

          {(run.tool_calls?.length ?? 0) > 0 && (
            <div>
              <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">
                Tool Calls ({run.tool_calls!.length})
              </div>
              <div className="space-y-1 max-h-64 overflow-auto">
                {run.tool_calls!.map((tc, i) => (
                  <div key={i} className={`rounded border p-2 ${tc.is_error ? "border-destructive/30" : ""}`}>
                    <div className="flex items-center gap-2">
                      <Badge variant={tc.is_error ? "destructive" : "secondary"} className="text-[10px]">{tc.tool}</Badge>
                      <span className="text-muted-foreground truncate flex-1">{tc.params_preview}</span>
                      <span className="text-muted-foreground">{(tc.duration_ms / 1000).toFixed(1)}s</span>
                    </div>
                    {tc.result_preview && (
                      <pre className="mt-1 text-[10px] bg-muted/50 rounded p-1.5 whitespace-pre-wrap max-h-24 overflow-auto font-mono">
                        {tc.result_preview}
                      </pre>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {run.diff && (
            <div>
              <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">Diff</div>
              <pre className="bg-muted/50 rounded p-2 font-mono whitespace-pre-wrap max-h-48 overflow-auto">
                {run.diff}
              </pre>
            </div>
          )}

          <div className="pt-1">
            <Link to={`/run/${encodeURIComponent(run.id ?? "")}`} className="text-primary text-xs hover:underline">
              Full detail &rarr;
            </Link>
          </div>
        </CardContent>
      )}
    </Card>
  );
}

export function TasksPage() {
  const { agents: live, loading: liveLoading } = useLiveAgents();
  const { runs: all, loading: allLoading } = useRuns("alpha-swarm2");

  const scheduled = all.filter((r) => r.status === "pending" || r.status === "planning" || r.status === "planned" || r.status === "approved");
  const running = live.length > 0 ? live : all.filter((r) => r.status === "running");
  const completed = all.filter((r) => r.status === "passed" || r.status === "failed" || r.status === "skipped");

  if (liveLoading && allLoading) return <p className="text-muted-foreground">Loading...</p>;

  return (
    <div className="space-y-8">
      <h1 className="text-2xl font-bold">Tasks</h1>

      {running.length > 0 && (
        <section>
          <h2 className="text-sm font-semibold text-yellow-500 mb-3">Running ({running.length})</h2>
          <div className="space-y-2">
            {running.map((r) => <TaskAccordion key={r.id} run={r} />)}
          </div>
        </section>
      )}

      {scheduled.length > 0 && (
        <section>
          <h2 className="text-sm font-semibold text-muted-foreground mb-3">Scheduled ({scheduled.length})</h2>
          <div className="space-y-2">
            {scheduled.map((r) => <TaskAccordion key={r.id} run={r} />)}
          </div>
        </section>
      )}

      {completed.length > 0 && (
        <section>
          <h2 className="text-sm font-semibold mb-3">Completed ({completed.length})</h2>
          <div className="space-y-2">
            {completed.map((r) => <TaskAccordion key={r.id} run={r} />)}
          </div>
        </section>
      )}

      {running.length === 0 && scheduled.length === 0 && completed.length === 0 && (
        <p className="text-muted-foreground">
          No tasks yet. <Link to="/submit" className="text-primary underline">Submit one</Link>.
        </p>
      )}
    </div>
  );
}
