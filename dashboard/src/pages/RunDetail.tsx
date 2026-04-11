import { useParams } from "react-router";
import { Card, CardContent } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { StatusBadge } from "@/components/StatusBadge";
import { Waterfall } from "@/components/Waterfall";
import { ToolCallList } from "@/components/ToolCallList";
import { AgentCard } from "@/components/AgentCard";
import { AttemptRow } from "@/components/AttemptRow";
import { useRunDetail } from "@/hooks/useRunDetail";
import type { AgentRun } from "@/types/swarm";

function RunHeader({ run }: { run: AgentRun }) {
  const duration = run.duration_ms > 0
    ? `${(run.duration_ms / 1000).toFixed(0)}s`
    : "running...";

  return (
    <div>
      <StatusBadge status={run.status} />
      <h1 className="text-xl font-bold mt-2">{run.task_description}</h1>
      <p className="text-xs text-muted-foreground mt-1 font-mono">
        {run.id} | {run.model_used} | {duration}
      </p>
    </div>
  );
}

function RunSections({ run, subRuns }: { run: AgentRun; subRuns: AgentRun[] }) {
  return (
    <>
      {run.progress_message && (
        <Card>
          <CardContent className="pt-4">
            <code className="text-xs whitespace-pre-wrap">{run.progress_message}</code>
          </CardContent>
        </Card>
      )}

      {run.error_message && (
        <Card className="border-destructive">
          <CardContent className="pt-4 text-destructive text-sm">{run.error_message}</CardContent>
        </Card>
      )}

      {run.phase_timings && (
        <div>
          <h3 className="text-sm font-semibold mb-3">Phase Timings</h3>
          <Waterfall timings={run.phase_timings} totalMs={run.duration_ms || undefined} />
        </div>
      )}

      {subRuns.length > 0 && (
        <div>
          <Separator className="mb-4" />
          <h3 className="text-sm font-semibold mb-3">Sub-agents ({subRuns.length})</h3>
          <div className="space-y-2">
            {subRuns.map((sub) => (
              <AgentCard key={sub.id} run={sub} />
            ))}
          </div>
        </div>
      )}

      {run.tool_calls.length > 0 && (
        <div>
          <Separator className="mb-4" />
          <h3 className="text-sm font-semibold mb-3">Tool Calls ({run.tool_calls.length})</h3>
          <ToolCallList calls={run.tool_calls} />
        </div>
      )}

      {run.attempts.length > 0 && (
        <div>
          <Separator className="mb-4" />
          <h3 className="text-sm font-semibold mb-3">Attempts ({run.attempts.length})</h3>
          {run.attempts.map((a, i) => <AttemptRow key={i} attempt={a} />)}
        </div>
      )}

      {run.diff && (
        <div>
          <Separator className="mb-4" />
          <h3 className="text-sm font-semibold mb-3">Diff</h3>
          <pre className="bg-muted/50 rounded p-3 text-xs font-mono whitespace-pre-wrap max-h-96 overflow-auto">
            {run.diff}
          </pre>
        </div>
      )}
    </>
  );
}

export function RunDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { run, subRuns, loading, error } = useRunDetail(id ?? "");

  if (!id) return <p className="text-muted-foreground">No run ID</p>;
  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;
  if (!run) return <p className="text-muted-foreground">Run not found</p>;

  return (
    <div className="space-y-6">
      <RunHeader run={run} />
      <RunSections run={run} subRuns={subRuns} />
    </div>
  );
}
