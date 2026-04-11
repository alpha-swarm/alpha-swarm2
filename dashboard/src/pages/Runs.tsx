import { useParams, Link, useNavigate } from "react-router";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Button } from "@/components/ui/button";
import { StatusBadge } from "@/components/StatusBadge";
import { MetricsGrid } from "@/components/MetricsGrid";
import { useRuns } from "@/hooks/useRuns";
import { useProjectMetrics } from "@/hooks/useProjectMetrics";
import type { AgentRun } from "@/types/swarm";
import type { ProjectMetrics } from "@/hooks/useProjectMetrics";

const TASK_PREVIEW_LENGTH = 80;

function buildMetricItems(metrics: ProjectMetrics) {
  return [
    { label: "Total", value: metrics.total_runs, className: "" },
    { label: "Active", value: metrics.active, className: "text-yellow-500" },
    { label: "Passed", value: metrics.passed, className: "text-green-500" },
    { label: "Failed", value: metrics.failed, className: "text-destructive" },
    { label: "Pending", value: metrics.pending, className: "text-muted-foreground" },
  ];
}

function formatDuration(ms: number) { return ms > 0 ? `${(ms / 1000).toFixed(0)}s` : "-"; }
function formatQg(v: boolean | null) { return v === true ? "pass" : v === false ? "fail" : "-"; }

function RunRow({ run }: { run: AgentRun }) {
  const desc = run.task_description.slice(0, TASK_PREVIEW_LENGTH);
  return (
    <TableRow>
      <TableCell><StatusBadge status={run.status} /></TableCell>
      <TableCell>
        <Link to={`/run/${encodeURIComponent(run.id ?? "")}`} className="text-primary hover:underline">{desc}{run.task_description.length > TASK_PREVIEW_LENGTH ? "..." : ""}</Link>
        {run.progress_message && <p className="text-xs text-muted-foreground mt-0.5 truncate max-w-md">{run.progress_message}</p>}
      </TableCell>
      <TableCell className="text-xs font-mono">{run.model_used}</TableCell>
      <TableCell className="text-xs">{formatDuration(run.duration_ms)}</TableCell>
      <TableCell className="text-xs">{formatQg(run.quality_gate_passed)}</TableCell>
    </TableRow>
  );
}

function RunsHeader({ project, count }: { project: string; count: number }) {
  const navigate = useNavigate();
  return (
    <div className="flex items-center justify-between mb-6">
      <div>
        <h1 className="text-2xl font-bold">{project}</h1>
        <p className="text-sm text-muted-foreground">{count} runs</p>
      </div>
      <Button onClick={() => navigate("/submit")}>New Task</Button>
    </div>
  );
}

function RunsTable({ runs }: { runs: AgentRun[] }) {
  if (runs.length === 0) return <p className="text-muted-foreground">No runs yet.</p>;
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead className="w-24">Status</TableHead>
          <TableHead>Task</TableHead>
          <TableHead className="w-40">Model</TableHead>
          <TableHead className="w-24">Duration</TableHead>
          <TableHead className="w-16">QG</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>{runs.map((r) => <RunRow key={r.id} run={r} />)}</TableBody>
    </Table>
  );
}

function ProjectMetrics({ project }: { project: string }) {
  const { metrics } = useProjectMetrics(project);
  if (!metrics) return null;
  return <div className="mb-6"><MetricsGrid items={buildMetricItems(metrics)} compact /></div>;
}

export function RunsPage() {
  const { project } = useParams<{ project: string }>();
  const { runs, loading, error } = useRuns(project ?? "");

  if (!project) return <p className="text-muted-foreground">No project selected</p>;
  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;

  return (
    <div>
      <RunsHeader project={project} count={runs.length} />
      <ProjectMetrics project={project} />
      <RunsTable runs={runs} />
    </div>
  );
}
