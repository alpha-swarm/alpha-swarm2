import { useState, useEffect } from "react";
import { Link } from "react-router";
import { StatusBadge } from "@/components/StatusBadge";
import { LiveToolStream } from "@/components/LiveToolStream";
import { Waterfall } from "@/components/Waterfall";
import { resources } from "@/lib/mcp";
import type { AgentRun, RunStatus } from "@/types/swarm";

const POLL_MS = 3_000;

const BORDER: Record<string, string> = {
  running: "border-l-amber-400",
  passed: "border-l-emerald-500",
  failed: "border-l-red-500",
  planning: "border-l-violet-400",
};

function borderCls(s: RunStatus) {
  return `border-l-[3px] ${BORDER[s] ?? "border-l-border"}`;
}

function formatDur(ms: number) {
  if (ms <= 0) return "...";
  if (ms < 60_000) return `${(ms / 1000).toFixed(0)}s`;
  return `${(ms / 60_000).toFixed(1)}m`;
}

function Accordion({ open, onToggle, children, header }: { open: boolean; onToggle: () => void; children: React.ReactNode; header: React.ReactNode }) {
  return (
    <div>
      <button onClick={onToggle} className="w-full text-left flex items-center gap-2">
        <span className="text-muted-foreground/40 text-[10px] w-3">{open ? "\u25BC" : "\u25B6"}</span>
        {header}
      </button>
      {open && <div className="ml-5 mt-1">{children}</div>}
    </div>
  );
}

function AgentHeader({ run }: { run: AgentRun }) {
  return (
    <div className="flex items-center gap-2 flex-1 min-w-0">
      <StatusBadge status={run.status} />
      <span className="text-xs truncate flex-1">{run.task_description.slice(0, 80)}</span>
      <span className="text-[10px] font-mono tabular-nums text-muted-foreground/50 shrink-0">{formatDur(run.duration_ms)}</span>
    </div>
  );
}

function AgentBody({ run }: { run: AgentRun }) {
  const tools = run.tool_calls ?? [];
  return (
    <div className="space-y-1 py-1">
      {run.progress_message && <div className="font-mono text-[11px] text-muted-foreground/70 bg-muted/30 rounded px-2 py-1 truncate">{run.progress_message}</div>}
      {run.error_message && <div className="text-[11px] text-red-400 bg-red-400/10 rounded px-2 py-1">{run.error_message}</div>}
      {tools.length > 0 && <LiveToolStream calls={tools} />}
    </div>
  );
}

function SubAgentAccordion({ run }: { run: AgentRun }) {
  const active = run.status === "running";
  const [open, setOpen] = useState(active);
  return (
    <div className="border-l border-border/30 pl-2">
      <Accordion open={open} onToggle={() => setOpen(!open)} header={<AgentHeader run={run} />}>
        <AgentBody run={run} />
      </Accordion>
    </div>
  );
}

function GoalBody({ run, subs }: { run: AgentRun; subs: AgentRun[] }) {
  return (
    <div className="space-y-1.5">
      {run.phase_timings && <Waterfall timings={run.phase_timings} totalMs={run.duration_ms || undefined} />}
      <AgentBody run={run} />
      {subs.map((s) => <SubAgentAccordion key={s.id} run={s} />)}
      {run.diff && <DiffDetails diff={run.diff} />}
      <DetailLink id={run.id} />
    </div>
  );
}

function DiffDetails({ diff }: { diff: string }) {
  return (
    <details><summary className="text-[10px] text-muted-foreground/50 cursor-pointer">diff ({diff.split('\n').length} lines)</summary>
      <pre className="text-[10px] font-mono bg-muted/20 rounded p-2 mt-1 max-h-32 overflow-auto whitespace-pre-wrap">{diff}</pre>
    </details>
  );
}

function DetailLink({ id }: { id: string | null }) {
  if (!id) return null;
  return <Link to={`/run/${encodeURIComponent(id)}`} className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground">full detail &rarr;</Link>;
}

function useGoalDetail(open: boolean, id: string | null) {
  const [detail, setDetail] = useState<AgentRun | null>(null);
  const [subs, setSubs] = useState<AgentRun[]>([]);
  useEffect(() => {
    if (!open || !id) return;
    const f = async () => {
      try {
        const [d, s] = await Promise.all([resources.runDetail(id), resources.subRuns(id)]);
        if (d[0]) setDetail(d[0]);
        setSubs(s);
      } catch { /* */ }
    };
    f();
    const iv = setInterval(f, POLL_MS);
    return () => clearInterval(iv);
  }, [open, id]);
  return { detail, subs };
}

export function GoalCard({ run }: { run: AgentRun }) {
  const active = run.status === "running" || run.status === "planning";
  const [open, setOpen] = useState(active);
  const { detail, subs } = useGoalDetail(open, run.id);
  const full = detail ?? run;

  return (
    <div className={`${borderCls(full.status)} ${active ? "animate-pulse" : ""} rounded-r bg-card/30 hover:bg-card/50 transition-colors px-3 py-2`}>
      <Accordion open={open} onToggle={() => setOpen(!open)} header={<AgentHeader run={full} />}>
        <GoalBody run={full} subs={subs} />
      </Accordion>
    </div>
  );
}
