import { useState } from "react";
import { Link } from "react-router";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/StatusBadge";
import { MetricsGrid } from "@/components/MetricsGrid";
import { useDashboard } from "@/hooks/useDashboard";
import { useLiveAgents } from "@/hooks/useLiveAgents";
import { useRuns } from "@/hooks/useRuns";
import type { DashboardStats, AgentRun } from "@/types/swarm";

function buildItems(s: DashboardStats) {
  return [
    { label: "Active", value: s.active, className: "text-yellow-500" },
    { label: "Passed", value: s.passed, className: "text-green-500" },
    { label: "Failed", value: s.failed, className: "text-destructive" },
    { label: "Pending", value: s.pending, className: "text-muted-foreground" },
    { label: "Total", value: s.total_runs, className: "" },
  ];
}

function TaskRow({ run }: { run: AgentRun }) {
  const [open, setOpen] = useState(run.status === "running");
  return (
    <div className={`rounded-md border ${run.status === "running" ? "border-yellow-500/30" : ""}`}>
      <TaskRowHeader run={run} open={open} onToggle={() => setOpen(!open)} />
      {!open && run.progress_message && <div className="px-3 pb-2 text-xs text-muted-foreground font-mono truncate">{run.progress_message}</div>}
      {open && <TaskRowBody run={run} />}
    </div>
  );
}

function TaskRowHeader({ run, open, onToggle }: { run: AgentRun; open: boolean; onToggle: () => void }) {
  return (
    <button onClick={onToggle} className="w-full flex items-center gap-3 px-3 py-2.5 text-left hover:bg-muted/30 transition-colors text-sm">
      <StatusBadge status={run.status} />
      <span className="flex-1 truncate">{run.task_description.slice(0, 80)}</span>
      <span className="text-xs text-muted-foreground">{run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "..."}</span>
      <span className="text-xs text-muted-foreground">{open ? "\u25B2" : "\u25BC"}</span>
    </button>
  );
}

function TaskRowBody({ run }: { run: AgentRun }) {
  const tools = run.tool_calls ?? [];
  return (
    <div className="px-3 pb-3 space-y-2 text-xs border-t">
      <div className="grid grid-cols-3 gap-2 pt-2">
        <div><span className="text-muted-foreground">ID:</span> <span className="font-mono">{run.id}</span></div>
        <div><span className="text-muted-foreground">Model:</span> {run.model_used}</div>
        <div><span className="text-muted-foreground">Agent:</span> {run.agent_id}</div>
      </div>
      {run.progress_message && <pre className="bg-muted/50 rounded p-2 font-mono whitespace-pre-wrap">{run.progress_message}</pre>}
      {run.error_message && <div className="text-destructive bg-destructive/10 rounded p-2">{run.error_message}</div>}
      <ToolBadges timings={run.phase_timings} />
      <ToolCallPreview tools={tools} />
      <Link to={`/run/${encodeURIComponent(run.id ?? "")}`} className="text-primary hover:underline">Full detail &rarr;</Link>
    </div>
  );
}

function ToolBadges({ timings }: { timings?: AgentRun["phase_timings"] }) {
  if (!timings) return null;
  return (
    <div className="flex gap-2 flex-wrap">
      {Object.entries(timings).map(([k, v]) => (
        <Badge key={k} variant="outline" className="text-[10px]">{k.replace("_ms", "")}: {((v as number) / 1000).toFixed(1)}s</Badge>
      ))}
    </div>
  );
}

function ToolCallPreview({ tools }: { tools: NonNullable<AgentRun["tool_calls"]> }) {
  if (tools.length === 0) return null;
  return (
    <div className="space-y-1 max-h-48 overflow-auto">
      {tools.map((tc, i) => (
        <div key={i} className={`rounded border p-1.5 ${tc.is_error ? "border-destructive/30" : ""}`}>
          <div className="flex items-center gap-2">
            <Badge variant={tc.is_error ? "destructive" : "secondary"} className="text-[10px]">{tc.tool}</Badge>
            <span className="text-muted-foreground truncate flex-1">{tc.params_preview}</span>
            <span className="text-muted-foreground">{(tc.duration_ms / 1000).toFixed(1)}s</span>
          </div>
        </div>
      ))}
    </div>
  );
}

function Section({ title, runs, color }: { title: string; runs: AgentRun[]; color?: string }) {
  if (runs.length === 0) return null;
  return (
    <div>
      <h3 className={`text-sm font-semibold mb-2 ${color ?? ""}`}>{title} ({runs.length})</h3>
      <div className="space-y-1.5">{runs.map((r) => <TaskRow key={r.id} run={r} />)}</div>
    </div>
  );
}

export function DashboardPage() {
  const { stats, loading, error } = useDashboard();
  const { agents: live } = useLiveAgents();
  const { runs: all } = useRuns("alpha-swarm2");

  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;

  const running = live.length > 0 ? live : all.filter((r) => r.status === "running");
  const scheduled = all.filter((r) => ["pending", "planning", "planned", "approved"].includes(r.status));
  const completed = all.filter((r) => ["passed", "failed", "skipped"].includes(r.status));

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">Dashboard</h1>
      {stats && <MetricsGrid items={buildItems(stats)} columns="grid-cols-2 md:grid-cols-3 lg:grid-cols-5" />}
      <Section title="Running" runs={running} color="text-yellow-500" />
      <Section title="Scheduled" runs={scheduled} color="text-muted-foreground" />
      <Section title="Completed" runs={completed} />
    </div>
  );
}
