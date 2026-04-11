import { useState, useEffect } from "react";
import { Link } from "react-router";
import { StatusBadge } from "@/components/StatusBadge";
import { LiveToolStream } from "@/components/LiveToolStream";
import { Waterfall } from "@/components/Waterfall";
import { resources } from "@/lib/mcp";
import type { AgentRun, RunStatus } from "@/types/swarm";

const POLL_MS = 3_000;

const BORDER_COLOR: Record<string, string> = {
  running: "border-l-amber-400",
  passed: "border-l-emerald-500",
  failed: "border-l-red-500",
  planning: "border-l-violet-400",
  pending: "border-l-muted",
};

function borderClass(status: RunStatus) {
  return `border-l-[3px] ${BORDER_COLOR[status] ?? "border-l-border"}`;
}

function isActive(status: RunStatus) {
  return status === "running" || status === "planning";
}

function formatDur(ms: number) {
  if (ms <= 0) return "...";
  if (ms < 60_000) return `${(ms / 1000).toFixed(0)}s`;
  return `${(ms / 60_000).toFixed(1)}m`;
}

function GoalHeader({ run, expanded }: { run: AgentRun; expanded: boolean }) {
  return (
    <div className="flex items-center gap-2.5">
      <span className="text-muted-foreground/40 text-[10px]">{expanded ? "\u25BC" : "\u25B6"}</span>
      <StatusBadge status={run.status} />
      <span className="text-sm truncate flex-1">{run.task_description}</span>
      <span className="text-xs font-mono tabular-nums text-muted-foreground/60 shrink-0">{formatDur(run.duration_ms)}</span>
    </div>
  );
}

function ProgressLine({ message }: { message: string }) {
  return (
    <div className="font-mono text-[11px] text-muted-foreground/70 bg-muted/30 rounded px-2 py-1 mt-2 truncate">
      {message}
    </div>
  );
}

function ErrorBlock({ message }: { message: string }) {
  return <div className="text-[11px] text-red-400 bg-red-400/10 rounded px-2 py-1 mt-2">{message}</div>;
}

function GoalMessages({ run }: { run: AgentRun }) {
  return (
    <>
      {run.progress_message && <ProgressLine message={run.progress_message} />}
      {run.error_message && <ErrorBlock message={run.error_message} />}
    </>
  );
}

function GoalBody({ run, subRuns }: { run: AgentRun; subRuns: AgentRun[] }) {
  const tools = run.tool_calls ?? [];
  return (
    <div className="mt-2 space-y-1.5">
      {run.phase_timings && <Waterfall timings={run.phase_timings} totalMs={run.duration_ms || undefined} />}
      <GoalMessages run={run} />
      {tools.length > 0 && <LiveToolStream calls={tools} />}
      {subRuns.map((s) => <SubAgent key={s.id} run={s} />)}
      {run.diff && <DiffPreview diff={run.diff} />}
      <DetailLink id={run.id} />
    </div>
  );
}

function SubAgent({ run }: { run: AgentRun }) {
  const tools = run.tool_calls ?? [];
  return (
    <div className="border-l border-border/30 ml-1 pl-2.5 py-1">
      <div className="flex items-center gap-2 text-xs">
        <StatusBadge status={run.status} />
        <span className="truncate text-muted-foreground/70">{run.task_description.slice(0, 60)}</span>
      </div>
      {run.progress_message && <ProgressLine message={run.progress_message} />}
      {tools.length > 0 && <LiveToolStream calls={tools} />}
    </div>
  );
}

function DiffPreview({ diff }: { diff: string }) {
  return (
    <details className="mt-1">
      <summary className="text-[10px] text-muted-foreground/50 cursor-pointer hover:text-muted-foreground">diff ({diff.split('\n').length} lines)</summary>
      <pre className="text-[10px] font-mono bg-muted/20 rounded p-2 mt-1 max-h-32 overflow-auto whitespace-pre-wrap">{diff}</pre>
    </details>
  );
}

function DetailLink({ id }: { id: string | null }) {
  if (!id) return null;
  return <Link to={`/run/${encodeURIComponent(id)}`} className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground">full detail &rarr;</Link>;
}

function useGoalDetail(expanded: boolean, id: string | null) {
  const [detail, setDetail] = useState<AgentRun | null>(null);
  const [subs, setSubs] = useState<AgentRun[]>([]);
  useEffect(() => {
    if (!expanded || !id) return;
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
  }, [expanded, id]);
  return { detail, subs };
}

export function GoalCard({ run }: { run: AgentRun }) {
  const autoExpand = isActive(run.status);
  const [expanded, setExpanded] = useState(autoExpand);
  const { detail, subs } = useGoalDetail(expanded, run.id);
  const full = detail ?? run;
  const pulse = isActive(full.status) ? "animate-pulse" : "";

  return (
    <div
      className={`${borderClass(full.status)} ${pulse} rounded-r bg-card/30 hover:bg-card/50 transition-colors cursor-pointer`}
      onClick={() => setExpanded(!expanded)}
    >
      <div className="px-3 py-2">
        <GoalHeader run={full} expanded={expanded} />
        {expanded && <div onClick={(e) => e.stopPropagation()}><GoalBody run={full} subRuns={subs} /></div>}
      </div>
    </div>
  );
}
