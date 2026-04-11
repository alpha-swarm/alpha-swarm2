import { useState } from "react";
import { StatPills } from "@/components/StatPills";
import { GoalCard } from "@/components/GoalCard";
import { useDashboard } from "@/hooks/useDashboard";
import { useLiveAgents } from "@/hooks/useLiveAgents";
import { useRuns } from "@/hooks/useRuns";
import { useProjects } from "@/hooks/useProjects";
import type { AgentRun } from "@/types/swarm";

function sortGoals(live: AgentRun[], all: AgentRun[]) {
  const running = live.length > 0 ? live : all.filter((r) => r.status === "running");
  const rest = all.filter((r) => !running.some((l) => l.id === r.id));
  return [...running, ...rest];
}

function ProjectSelector({ value, options, onChange }: { value: string; options: string[]; onChange: (v: string) => void }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} className="text-xs bg-transparent border rounded px-2 py-1">
      {options.map((p) => <option key={p} value={p}>{p}</option>)}
    </select>
  );
}

function GoalList({ goals }: { goals: AgentRun[] }) {
  if (goals.length === 0) return <p className="text-sm text-muted-foreground/50 py-8 text-center">No goals yet.</p>;
  return <div className="space-y-1">{goals.map((g) => <GoalCard key={g.id} run={g} />)}</div>;
}

export function DashboardPage() {
  const { projects } = useProjects();
  const projectNames = projects.map((p) => p.name);
  const [project, setProject] = useState("alpha-swarm2");

  const { stats, loading } = useDashboard();
  const { agents: live } = useLiveAgents();
  const { runs: all } = useRuns(project);

  if (loading) return <p className="text-sm text-muted-foreground/50">Loading...</p>;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        {stats && <StatPills stats={stats} />}
        {projectNames.length > 1 && <ProjectSelector value={project} options={projectNames} onChange={setProject} />}
      </div>
      <GoalList goals={sortGoals(live, all)} />
    </div>
  );
}
