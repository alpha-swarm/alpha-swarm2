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
  const duration = run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "running...";
  return (
    <div>
      <StatusBadge status={run.status} />
      <h1 className="text-xl font-bold mt-2">{run.task_description}</h1>
      <p className="text-xs text-muted-foreground mt-1 font-mono">{run.id} | {run.model_used} | {duration}</p>
    </div>
  );
}

function ProgressCard({ message }: { message: string }) {
  return <Card><CardContent className="pt-4"><code className="text-xs whitespace-pre-wrap">{message}</code></CardContent></Card>;
}

function ErrorCard({ message }: { message: string }) {
  return <Card className="border-destructive"><CardContent className="pt-4 text-destructive text-sm">{message}</CardContent></Card>;
}

function PhaseTimingsSection({ run }: { run: AgentRun }) {
  if (!run.phase_timings) return null;
  return (
    <div>
      <h3 className="text-sm font-semibold mb-3">Phase Timings</h3>
      <Waterfall timings={run.phase_timings} totalMs={run.duration_ms || undefined} />
    </div>
  );
}

function SubAgents({ runs }: { runs: AgentRun[] }) {
  if (runs.length === 0) return null;
  return (
    <div>
      <Separator className="mb-4" />
      <h3 className="text-sm font-semibold mb-3">Sub-agents ({runs.length})</h3>
      <div className="space-y-2">{runs.map((s) => <AgentCard key={s.id} run={s} />)}</div>
    </div>
  );
}

function ToolCalls({ calls }: { calls: AgentRun["tool_calls"] }) {
  if (!calls || calls.length === 0) return null;
  return (
    <div>
      <Separator className="mb-4" />
      <h3 className="text-sm font-semibold mb-3">Tool Calls ({calls.length})</h3>
      <ToolCallList calls={calls} />
    </div>
  );
}

function Attempts({ attempts }: { attempts: AgentRun["attempts"] }) {
  if (!attempts || attempts.length === 0) return null;
  return (
    <div>
      <Separator className="mb-4" />
      <h3 className="text-sm font-semibold mb-3">Attempts ({attempts.length})</h3>
      {attempts.map((a, i) => <AttemptRow key={i} attempt={a} />)}
    </div>
  );
}

function DiffBlock({ diff }: { diff: string }) {
  return (
    <div>
      <Separator className="mb-4" />
      <h3 className="text-sm font-semibold mb-3">Diff</h3>
      <pre className="bg-muted/50 rounded p-3 text-xs font-mono whitespace-pre-wrap max-h-96 overflow-auto">{diff}</pre>
    </div>
  );
}

function RunDetailContent({ run, subRuns }: { run: AgentRun; subRuns: AgentRun[] }) {
  return (
    <div className="space-y-6">
      <RunHeader run={run} />
      {run.progress_message && <ProgressCard message={run.progress_message} />}
      {run.error_message && <ErrorCard message={run.error_message} />}
      <PhaseTimingsSection run={run} />
      <SubAgents runs={subRuns} />
      <ToolCalls calls={run.tool_calls} />
      <Attempts attempts={run.attempts} />
      {run.diff && <DiffBlock diff={run.diff} />}
    </div>
  );
}

export function RunDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { run, subRuns, loading, error } = useRunDetail(id ?? "");

  if (!id) return <p className="text-muted-foreground">No run ID</p>;
  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;
  if (!run) return <p className="text-muted-foreground">Run not found</p>;

  return <RunDetailContent run={run} subRuns={subRuns} />;
}
