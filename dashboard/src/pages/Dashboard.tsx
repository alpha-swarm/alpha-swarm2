import { StatPills } from "@/components/StatPills";
import { GoalCard } from "@/components/GoalCard";
import { useDashboard } from "@/hooks/useDashboard";
import { useLiveAgents } from "@/hooks/useLiveAgents";
import { useRuns } from "@/hooks/useRuns";
import type { AgentRun } from "@/types/swarm";

function sortGoals(live: AgentRun[], all: AgentRun[]) {
  const running = live.length > 0 ? live : all.filter((r) => r.status === "running");
  const rest = all.filter((r) => !running.some((l) => l.id === r.id));
  return [...running, ...rest];
}

function GoalList({ goals }: { goals: AgentRun[] }) {
  if (goals.length === 0) return <EmptyState />;
  return (
    <div className="space-y-1">
      {goals.map((g) => <GoalCard key={g.id} run={g} />)}
    </div>
  );
}

function EmptyState() {
  return <p className="text-sm text-muted-foreground/50 py-8 text-center">No goals yet. Submit a task to get started.</p>;
}

export function DashboardPage() {
  const { stats, loading } = useDashboard();
  const { agents: live } = useLiveAgents();
  const { runs: all } = useRuns("alpha-swarm2");

  if (loading) return <p className="text-sm text-muted-foreground/50">Loading...</p>;

  const goals = sortGoals(live, all);

  return (
    <div className="space-y-3">
      {stats && <StatPills stats={stats} />}
      <GoalList goals={goals} />
    </div>
  );
}
