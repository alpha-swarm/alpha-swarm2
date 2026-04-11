import { useState, useEffect } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/StatusBadge";
import { ToolCallList } from "@/components/ToolCallList";
import { resources } from "@/lib/mcp";
import type { AgentRun } from "@/types/swarm";

const POLL_MS = 5_000;

interface Props { run: AgentRun; depth?: number; maxDepth?: number }

function useAgentDetail(expanded: boolean, id: string | null) {
  const [detail, setDetail] = useState<AgentRun | null>(null);
  const [subs, setSubs] = useState<AgentRun[]>([]);
  useEffect(() => {
    if (!expanded || !id) return;
    const f = async () => {
      try {
        const [d, s] = await Promise.all([resources.runDetail(id), resources.subRuns(id)]);
        if (d[0]) setDetail(d[0]);
        setSubs(s);
      } catch { /* ignore */ }
    };
    f();
    const iv = setInterval(f, POLL_MS);
    return () => clearInterval(iv);
  }, [expanded, id]);
  return { detail, subs };
}

function Header({ run, depth, maxDepth, expanded }: { run: AgentRun; depth: number; maxDepth: number; expanded: boolean }) {
  return (
    <div className="flex items-center gap-3">
      <StatusBadge status={run.status} />
      <CardTitle className="text-sm font-medium flex-1 truncate">{run.task_description.slice(0, 100)}</CardTitle>
      {depth > 0 && <Badge variant="outline" className="text-[9px]">depth {depth}/{maxDepth}</Badge>}
      <span className="text-xs text-muted-foreground">{run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "..."}</span>
      <span className="text-muted-foreground text-xs">{expanded ? "\u25B2" : "\u25BC"}</span>
    </div>
  );
}

function Body({ run, subs, depth, maxDepth }: { run: AgentRun; subs: AgentRun[]; depth: number; maxDepth: number }) {
  const tools = run.tool_calls ?? [];
  return (
    <CardContent className="space-y-3">
      {run.error_message && <div className="text-xs text-destructive bg-destructive/10 rounded p-2">{run.error_message}</div>}
      {run.diff && <details><summary className="text-[10px] text-muted-foreground cursor-pointer uppercase">Diff</summary><pre className="bg-muted/50 rounded p-2 text-[11px] font-mono whitespace-pre-wrap max-h-48 overflow-auto mt-1">{run.diff}</pre></details>}
      {tools.length > 0 && <ToolCallList calls={tools} />}
      {subs.length > 0 && <SubAgents subs={subs} depth={depth} maxDepth={maxDepth} />}
    </CardContent>
  );
}

function SubAgents({ subs, depth, maxDepth }: { subs: AgentRun[]; depth: number; maxDepth: number }) {
  return (
    <div className="mt-2 border-l-2 border-muted pl-1">
      {subs.map((s) => <AgentCard key={s.id} run={s} depth={depth + 1} maxDepth={maxDepth} />)}
    </div>
  );
}

function cardStyle(depth: number) {
  return { marginLeft: `${depth * 16}px`, opacity: Math.max(0.5, 1 - depth * 0.15) };
}

function cardBorder(status: string) {
  return status === "running" ? "border-yellow-500/30" : "";
}

export function AgentCard({ run, depth = 0, maxDepth = 3 }: Props) {
  const [expanded, setExpanded] = useState(run.status === "running");
  const { detail, subs } = useAgentDetail(expanded, run.id);
  const full = detail ?? run;
  return (
    <div style={cardStyle(depth)}>
      <Card className={cardBorder(run.status)}>
        <CardHeader className="pb-2 cursor-pointer" onClick={() => setExpanded(!expanded)}>
          <Header run={full} depth={depth} maxDepth={maxDepth} expanded={expanded} />
        </CardHeader>
        {expanded && <Body run={full} subs={subs} depth={depth} maxDepth={maxDepth} />}
      </Card>
    </div>
  );
}
