import { useState } from "react";
import { Link } from "react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/StatusBadge";
import { useLiveAgents } from "@/hooks/useLiveAgents";
import { useRuns } from "@/hooks/useRuns";
import type { AgentRun } from "@/types/swarm";

function AccordionHeader({ run, open, onToggle }: { run: AgentRun; open: boolean; onToggle: () => void }) {
  return (
    <CardHeader className="pb-2 cursor-pointer hover:bg-muted/30 transition-colors" onClick={onToggle}>
      <div className="flex items-center gap-3">
        <StatusBadge status={run.status} />
        <CardTitle className="text-sm font-medium flex-1 truncate">{run.task_description.slice(0, 100)}</CardTitle>
        <span className="text-xs text-muted-foreground">{run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "..."}</span>
        <span className="text-muted-foreground text-xs">{open ? "\u25B2" : "\u25BC"}</span>
      </div>
      {run.progress_message && <p className="text-xs text-muted-foreground mt-1 font-mono truncate">{run.progress_message}</p>}
    </CardHeader>
  );
}

function AccordionBody({ run }: { run: AgentRun }) {
  const tools = run.tool_calls ?? [];
  return (
    <CardContent className="space-y-3 text-xs">
      <TaskMeta run={run} />
      {run.error_message && <div className="text-destructive bg-destructive/10 rounded p-2">{run.error_message}</div>}
      {tools.length > 0 && <ToolList tools={tools} />}
      {run.diff && <DiffPreview diff={run.diff} />}
      <Link to={`/run/${encodeURIComponent(run.id ?? "")}`} className="text-primary text-xs hover:underline">Full detail &rarr;</Link>
    </CardContent>
  );
}

function TaskMeta({ run }: { run: AgentRun }) {
  return (
    <div className="grid grid-cols-3 gap-2">
      <div><span className="text-muted-foreground">ID:</span> <span className="font-mono">{run.id}</span></div>
      <div><span className="text-muted-foreground">Model:</span> {run.model_used}</div>
      <div><span className="text-muted-foreground">Agent:</span> {run.agent_id}</div>
    </div>
  );
}

function ToolList({ tools }: { tools: NonNullable<AgentRun["tool_calls"]> }) {
  return (
    <div>
      <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">Tool Calls ({tools.length})</div>
      <div className="space-y-1 max-h-64 overflow-auto">
        {tools.map((tc, i) => (
          <div key={i} className={`rounded border p-2 ${tc.is_error ? "border-destructive/30" : ""}`}>
            <div className="flex items-center gap-2">
              <Badge variant={tc.is_error ? "destructive" : "secondary"} className="text-[10px]">{tc.tool}</Badge>
              <span className="text-muted-foreground truncate flex-1">{tc.params_preview}</span>
              <span className="text-muted-foreground">{(tc.duration_ms / 1000).toFixed(1)}s</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function DiffPreview({ diff }: { diff: string }) {
  return (
    <div>
      <div className="text-[10px] text-muted-foreground mb-1 font-semibold uppercase">Diff</div>
      <pre className="bg-muted/50 rounded p-2 font-mono whitespace-pre-wrap max-h-48 overflow-auto">{diff}</pre>
    </div>
  );
}

function TaskAccordion({ run }: { run: AgentRun }) {
  const [open, setOpen] = useState(run.status === "running");
  return (
    <Card className={run.status === "running" ? "border-yellow-500/30" : ""}>
      <AccordionHeader run={run} open={open} onToggle={() => setOpen(!open)} />
      {open && <AccordionBody run={run} />}
    </Card>
  );
}

function TaskSection({ title, runs, color }: { title: string; runs: AgentRun[]; color?: string }) {
  if (runs.length === 0) return null;
  return (
    <section>
      <h2 className={`text-sm font-semibold mb-3 ${color ?? ""}`}>{title} ({runs.length})</h2>
      <div className="space-y-2">{runs.map((r) => <TaskAccordion key={r.id} run={r} />)}</div>
    </section>
  );
}

const SCHEDULED_STATUSES = new Set(["pending", "planning", "planned", "approved"]);
const COMPLETED_STATUSES = new Set(["passed", "failed", "skipped"]);

function useTaskGroups() {
  const { agents: live, loading: ll } = useLiveAgents();
  const { runs: all, loading: al } = useRuns("alpha-swarm2");
  const running = live.length > 0 ? live : all.filter((r) => r.status === "running");
  const scheduled = all.filter((r) => SCHEDULED_STATUSES.has(r.status));
  const completed = all.filter((r) => COMPLETED_STATUSES.has(r.status));
  return { running, scheduled, completed, loading: ll && al };
}

export function TasksPage() {
  const { running, scheduled, completed, loading } = useTaskGroups();

  if (loading) return <p className="text-muted-foreground">Loading...</p>;

  const empty = running.length === 0 && scheduled.length === 0 && completed.length === 0;

  return (
    <div className="space-y-8">
      <h1 className="text-2xl font-bold">Tasks</h1>
      <TaskSection title="Running" runs={running} color="text-yellow-500" />
      <TaskSection title="Scheduled" runs={scheduled} color="text-muted-foreground" />
      <TaskSection title="Completed" runs={completed} />
      {empty && <p className="text-muted-foreground">No tasks. <Link to="/submit" className="text-primary underline">Submit one</Link>.</p>}
    </div>
  );
}
