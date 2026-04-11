import { useState } from "react";
import { StatPills } from "@/components/StatPills";
import { GoalCard } from "@/components/GoalCard";
import { Button } from "@/components/ui/button";
import { useDashboard } from "@/hooks/useDashboard";
import { useLiveAgents } from "@/hooks/useLiveAgents";
import { useRuns } from "@/hooks/useRuns";
import { useProjects } from "@/hooks/useProjects";
import { createProject, deleteProject, submitTask } from "@/lib/mcp";
import type { AgentRun, Project } from "@/types/swarm";

function sortGoals(live: AgentRun[], all: AgentRun[]) {
  const running = live.length > 0 ? live : all.filter((r) => r.status === "running");
  const rest = all.filter((r) => !running.some((l) => l.id === r.id));
  return [...running, ...rest];
}

function CreateProjectForm({ onCreated }: { onCreated: () => void }) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const handle = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name || !url) return;
    await createProject(name, url);
    setName(""); setUrl(""); onCreated();
  };
  return (
    <form onSubmit={handle} className="flex gap-2 text-xs">
      <input value={name} onChange={(e) => setName(e.target.value)} placeholder="name" className="border rounded px-2 py-1 bg-transparent w-32" />
      <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="repo url or path" className="border rounded px-2 py-1 bg-transparent flex-1" />
      <Button size="sm" variant="outline" className="h-7 text-xs" type="submit" disabled={!name || !url}>Add</Button>
    </form>
  );
}

function ProjectHeader({ p, open }: { p: Project; open: boolean }) {
  return (
    <div className="flex items-center gap-2 flex-1 min-w-0">
      <span className="text-muted-foreground/40 text-[10px] w-3">{open ? "\u25BC" : "\u25B6"}</span>
      <span className="text-sm font-medium">{p.name}</span>
      <span className="text-[10px] text-muted-foreground/40 truncate">{p.repo_url}</span>
    </div>
  );
}

function NewGoalInput({ project }: { project: string }) {
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);
  const handle = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!goal.trim()) return;
    setBusy(true);
    try { await submitTask(project, goal); setGoal(""); } catch { /* */ }
    finally { setBusy(false); }
  };
  return (
    <form onSubmit={handle} className="flex gap-2 mb-2">
      <input value={goal} onChange={(e) => setGoal(e.target.value)} placeholder="Describe a goal for the agents..."
        className="flex-1 border rounded px-2 py-1 text-xs bg-transparent font-mono" disabled={busy} />
      <Button size="sm" variant="outline" className="h-7 text-xs" type="submit" disabled={busy || !goal.trim()}>
        {busy ? "..." : "Run"}
      </Button>
    </form>
  );
}

function ProjectGoals({ project }: { project: string }) {
  const { agents: live } = useLiveAgents();
  const { runs: all, loading } = useRuns(project);
  if (loading) return <p className="text-[11px] text-muted-foreground/40 py-2">Loading...</p>;
  const goals = sortGoals(live, all);
  return (
    <div>
      <NewGoalInput project={project} />
      {goals.length === 0 ? <p className="text-[11px] text-muted-foreground/40 py-2">No goals yet.</p> : (
        <div className="space-y-1">{goals.map((g) => <GoalCard key={g.id} run={g} />)}</div>
      )}
    </div>
  );
}

function ProjectAccordion({ p, onDelete }: { p: Project; onDelete: () => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="border rounded bg-card/20">
      <div className="flex items-center px-3 py-2 cursor-pointer hover:bg-card/40" onClick={() => setOpen(!open)}>
        <ProjectHeader p={p} open={open} />
        <Button size="sm" variant="ghost" className="h-6 text-[10px] text-muted-foreground/30 hover:text-red-400" onClick={(e) => { e.stopPropagation(); onDelete(); }}>delete</Button>
      </div>
      {open && <div className="px-3 pb-3"><ProjectGoals project={p.name} /></div>}
    </div>
  );
}

export function DashboardPage() {
  const { stats, loading } = useDashboard();
  const { projects, refetch } = useProjects();
  const [showAdd, setShowAdd] = useState(false);

  if (loading) return <p className="text-sm text-muted-foreground/50">Loading...</p>;

  const handleDelete = async (name: string) => {
    await deleteProject(name);
    refetch();
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        {stats && <StatPills stats={stats} />}
        <Button size="sm" variant="ghost" className="h-6 text-[10px]" onClick={() => setShowAdd(!showAdd)}>
          {showAdd ? "cancel" : "+ project"}
        </Button>
      </div>
      {showAdd && <CreateProjectForm onCreated={() => { refetch(); setShowAdd(false); }} />}
      <div className="space-y-2">
        {projects.map((p) => <ProjectAccordion key={p.name} p={p} onDelete={() => handleDelete(p.name)} />)}
      </div>
      {projects.length === 0 && <p className="text-sm text-muted-foreground/50 py-4 text-center">Add a project to get started.</p>}
    </div>
  );
}
