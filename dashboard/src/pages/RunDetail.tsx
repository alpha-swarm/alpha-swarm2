import { useParams, Link } from "react-router";
import { StatusBadge } from "@/components/StatusBadge";
import { Waterfall } from "@/components/Waterfall";
import { PlanView } from "@/components/PlanView";
import { LiveToolStream } from "@/components/LiveToolStream";
import { useRunDetail } from "@/hooks/useRunDetail";
import type { AgentRun } from "@/types/swarm";

function RunHeader({ run }: { run: AgentRun }) {
  const dur = run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "...";
  return (
    <div>
      <Link to="/" className="text-xs text-muted-foreground/50 hover:text-muted-foreground">&larr; back</Link>
      <div className="flex items-center gap-3 mt-3">
        <StatusBadge status={run.status} />
        <span className="text-xs font-mono text-muted-foreground/50">{dur}</span>
      </div>
      <h1 className="text-lg font-medium mt-2">{run.task_description}</h1>
      <p className="text-xs font-mono text-muted-foreground/40 mt-1">{run.id} &middot; {run.model_used}</p>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h2 className="text-xs font-semibold text-muted-foreground/60 uppercase tracking-wide mb-2">{title}</h2>
      {children}
    </div>
  );
}

function SubAgentDetail({ run }: { run: AgentRun }) {
  const tools = run.tool_calls ?? [];
  return (
    <div className="border rounded p-3 space-y-2">
      <div className="flex items-center gap-2">
        <StatusBadge status={run.status} />
        <span className="text-sm">{run.task_description}</span>
      </div>
      {run.progress_message && <pre className="text-[11px] font-mono bg-muted/30 rounded p-2">{run.progress_message}</pre>}
      {tools.length > 0 && <LiveToolStream calls={tools} />}
    </div>
  );
}

function DiffBlock({ diff }: { diff: string }) {
  return <pre className="text-xs font-mono bg-muted/20 rounded p-4 max-h-[60vh] overflow-auto whitespace-pre-wrap">{diff}</pre>;
}

function ErrorSection({ msg }: { msg?: string | null }) {
  if (!msg) return null;
  return <div className="text-sm text-red-400 bg-red-400/10 rounded p-3">{msg}</div>;
}

function TimingsSection({ run }: { run: AgentRun }) {
  if (!run.phase_timings) return null;
  return <Section title="Phase Timings"><Waterfall timings={run.phase_timings} totalMs={run.duration_ms || undefined} /></Section>;
}

function PlanSection({ id }: { id: string | null }) {
  if (!id) return null;
  return <Section title="Plan"><PlanView runId={id} /></Section>;
}

function ToolsSection({ run }: { run: AgentRun }) {
  const tools = run.tool_calls ?? [];
  if (tools.length === 0) return null;
  return <Section title="Tool Calls"><LiveToolStream calls={tools} /></Section>;
}

function SubsSection({ subs }: { subs: AgentRun[] }) {
  if (subs.length === 0) return null;
  return <Section title="Sub-Agents">{subs.map((s) => <SubAgentDetail key={s.id} run={s} />)}</Section>;
}

function DiffSection({ diff }: { diff?: string | null }) {
  if (!diff) return null;
  return <Section title="Diff"><DiffBlock diff={diff} /></Section>;
}

export function RunDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { run, subRuns, loading, error } = useRunDetail(id ?? "");

  if (!id) return <p className="text-muted-foreground">No run ID</p>;
  if (loading) return <p className="text-muted-foreground/50 text-sm">Loading...</p>;
  if (error) return <p className="text-destructive text-sm">Error: {error}</p>;
  if (!run) return <p className="text-muted-foreground/50 text-sm">Run not found</p>;

  return (
    <div className="space-y-8 max-w-3xl mx-auto py-4">
      <RunHeader run={run} />
      <ErrorSection msg={run.error_message} />
      <TimingsSection run={run} />
      <PlanSection id={run.id} />
      <ToolsSection run={run} />
      <SubsSection subs={subRuns} />
      <DiffSection diff={run.diff} />
    </div>
  );
}
