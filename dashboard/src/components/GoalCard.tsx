import { useState, useEffect } from "react";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { StatusBadge } from "@/components/StatusBadge";
import { LiveToolStream } from "@/components/LiveToolStream";
import { PlanView } from "@/components/PlanView";
import { Waterfall } from "@/components/Waterfall";
import { approveTask, deleteTask, planFeedback } from "@/lib/mcp";
import { resources } from "@/lib/mcp";
import type { AgentRun, RunStatus } from "@/types/swarm";

const POLL_MS = 3_000;

const BORDER: Record<string, string> = {
  running: "border-l-amber-400",
  passed: "border-l-emerald-500",
  failed: "border-l-red-500",
  planning: "border-l-violet-400",
  planned: "border-l-blue-400",
};

function borderCls(s: RunStatus) {
  return `border-l-[3px] ${BORDER[s] ?? "border-l-border"}`;
}

function formatDur(ms: number) {
  if (ms <= 0) return "...";
  return ms < 60_000 ? `${(ms / 1000).toFixed(0)}s` : `${(ms / 60_000).toFixed(1)}m`;
}

function isLive(s: RunStatus) {
  return s === "running" || s === "planning";
}

function needsReview(s: RunStatus) {
  return s === "planned";
}

function isDone(s: RunStatus) {
  return s === "passed" || s === "failed" || s === "skipped";
}

function Header({ run, open }: { run: AgentRun; open: boolean }) {
  return (
    <div className="flex items-center gap-2.5">
      <span className="text-muted-foreground/40 text-[10px] w-3">{open ? "\u25BC" : "\u25B6"}</span>
      <StatusBadge status={run.status} />
      <span className="text-sm truncate flex-1">{run.task_description}</span>
      <span className="text-xs font-mono tabular-nums text-muted-foreground/60 shrink-0">{formatDur(run.duration_ms)}</span>
    </div>
  );
}

function ReviewActions({ runId }: { runId: string }) {
  const [fb, setFb] = useState("");
  const [busy, setBusy] = useState(false);
  const act = async (fn: () => Promise<unknown>) => { setBusy(true); try { await fn(); } catch {} finally { setBusy(false); } };
  return (
    <div className="flex flex-wrap gap-2 items-center mt-2">
      <Button size="sm" className="h-6 text-xs" disabled={busy} onClick={() => act(() => approveTask(runId))}>Start</Button>
      <Button size="sm" variant="destructive" className="h-6 text-xs" disabled={busy} onClick={() => act(() => deleteTask(runId))}>Delete</Button>
      <input value={fb} onChange={(e) => setFb(e.target.value)} placeholder="Feedback to re-plan..." className="flex-1 border rounded px-2 py-0.5 text-xs bg-transparent" />
      <Button size="sm" variant="outline" className="h-6 text-xs" disabled={busy || !fb} onClick={() => act(async () => { await planFeedback(runId, fb); setFb(""); })}>Re-plan</Button>
    </div>
  );
}

function Messages({ run }: { run: AgentRun }) {
  return (
    <>
      {run.progress_message && <div className="font-mono text-[11px] text-muted-foreground/70 bg-muted/30 rounded px-2 py-1 truncate">{run.progress_message}</div>}
      {run.error_message && <div className="text-[11px] text-red-400 bg-red-400/10 rounded px-2 py-1">{run.error_message}</div>}
    </>
  );
}

function SubAgent({ run }: { run: AgentRun }) {
  const [open, setOpen] = useState(run.status === "running");
  const tools = run.tool_calls ?? [];
  return (
    <div className="border-l border-border/30 pl-2">
      <button onClick={() => setOpen(!open)} className="w-full text-left flex items-center gap-2 text-xs">
        <span className="text-muted-foreground/40 text-[10px] w-3">{open ? "\u25BC" : "\u25B6"}</span>
        <StatusBadge status={run.status} />
        <span className="truncate flex-1 text-muted-foreground/70">{run.task_description.slice(0, 60)}</span>
      </button>
      {open && <div className="ml-5 py-1"><Messages run={run} />{tools.length > 0 && <LiveToolStream calls={tools} />}</div>}
    </div>
  );
}

function Timings({ run }: { run: AgentRun }) {
  if (!run.phase_timings) return null;
  return <Waterfall timings={run.phase_timings} totalMs={run.duration_ms || undefined} />;
}

function Plan({ run }: { run: AgentRun }) {
  if (!run.id) return null;
  return (
    <>
      <PlanView runId={run.id} />
      {needsReview(run.status) && <ReviewActions runId={run.id} />}
    </>
  );
}

function Tools({ run, subs }: { run: AgentRun; subs: AgentRun[] }) {
  const tools = run.tool_calls ?? [];
  return (
    <>
      {tools.length > 0 && <LiveToolStream calls={tools} />}
      {subs.map((s) => <SubAgent key={s.id} run={s} />)}
    </>
  );
}

function Body({ run, subs }: { run: AgentRun; subs: AgentRun[] }) {
  return (
    <div className="space-y-1.5 mt-2">
      <Timings run={run} />
      <Plan run={run} />
      <Messages run={run} />
      <Tools run={run} subs={subs} />
      {run.diff && <Diff diff={run.diff} />}
      {run.id && isDone(run.status) && <Link to={`/run/${encodeURIComponent(run.id)}`} className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground">full detail &rarr;</Link>}
    </div>
  );
}

function Diff({ diff }: { diff: string }) {
  return (
    <details><summary className="text-[10px] text-muted-foreground/50 cursor-pointer">diff ({diff.split('\n').length} lines)</summary>
      <pre className="text-[10px] font-mono bg-muted/20 rounded p-2 mt-1 max-h-32 overflow-auto whitespace-pre-wrap">{diff}</pre>
    </details>
  );
}

function useDetail(open: boolean, id: string | null) {
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
  const autoOpen = isLive(run.status) || needsReview(run.status);
  const [open, setOpen] = useState(autoOpen);
  const { detail, subs } = useDetail(open, run.id);
  const full = detail ?? run;

  return (
    <div className={`${borderCls(full.status)} ${isLive(full.status) ? "animate-pulse" : ""} rounded-r bg-card/30 hover:bg-card/50 transition-colors`}>
      <div className="px-3 py-2">
        <button onClick={() => setOpen(!open)} className="w-full text-left"><Header run={full} open={open} /></button>
        {open && <Body run={full} subs={subs} />}
      </div>
    </div>
  );
}
