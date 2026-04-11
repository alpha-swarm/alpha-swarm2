import { useState, useEffect } from "react";
import { Badge } from "@/components/ui/badge";
import { resources } from "@/lib/mcp";
import type { GoalPlan } from "@/types/swarm";

function PlanHeader({ plan, open }: { plan: GoalPlan; open: boolean }) {
  return (
    <div className="flex items-center gap-2 flex-1 min-w-0">
      <span className="text-muted-foreground/40 text-[10px] w-3">{open ? "\u25BC" : "\u25B6"}</span>
      <Badge variant="outline" className="text-[9px] px-1 h-4">v{plan.version}</Badge>
      <Badge variant={plan.status === "approved" ? "default" : "secondary"} className="text-[9px] px-1 h-4">{plan.status}</Badge>
      <span className="text-[10px] text-muted-foreground/50">{plan.sub_tasks.length} tasks</span>
      <span className="text-[10px] font-mono text-muted-foreground/40">{plan.model_used}</span>
    </div>
  );
}

function SubTaskLine({ task }: { task: GoalPlan["sub_tasks"][0] }) {
  return (
    <div className="flex items-center gap-2 py-0.5 text-[11px]">
      <Badge variant="outline" className="text-[8px] px-1 h-3.5 font-mono">{task.id}</Badge>
      <span className="truncate flex-1 text-muted-foreground/70">{task.description}</span>
      <span className="text-[9px] text-muted-foreground/40 shrink-0">{task.files.length} files</span>
      <Badge variant="secondary" className="text-[8px] px-1 h-3.5">{task.complexity}</Badge>
    </div>
  );
}

function PlanBody({ plan }: { plan: GoalPlan }) {
  return (
    <div className="ml-5 mt-1 space-y-0.5">
      {plan.sub_tasks.map((t) => <SubTaskLine key={t.id} task={t} />)}
      {plan.user_feedback && <FeedbackBlock feedback={plan.user_feedback} />}
      {plan.reasoning && <div className="text-[10px] text-muted-foreground/40 mt-1">{plan.reasoning}</div>}
    </div>
  );
}

function FeedbackBlock({ feedback }: { feedback: string }) {
  return <div className="text-[10px] text-amber-400/80 bg-amber-400/10 rounded px-2 py-1 mt-1">Feedback: {feedback}</div>;
}

function PlanAccordion({ plan }: { plan: GoalPlan }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button onClick={() => setOpen(!open)} className="w-full text-left flex items-center">
        <PlanHeader plan={plan} open={open} />
      </button>
      {open && <PlanBody plan={plan} />}
    </div>
  );
}

export function PlanView({ runId }: { runId: string }) {
  const [plans, setPlans] = useState<GoalPlan[]>([]);
  useEffect(() => {
    if (!runId) return;
    resources.plans(runId).then((p) => setPlans(p)).catch(() => {});
  }, [runId]);

  if (plans.length === 0) return null;

  return (
    <div className="border-l border-violet-400/30 pl-2.5 space-y-1">
      <span className="text-[10px] text-muted-foreground/40 uppercase font-semibold">Plans ({plans.length})</span>
      {plans.map((p, i) => <PlanAccordion key={i} plan={p} />)}
    </div>
  );
}
