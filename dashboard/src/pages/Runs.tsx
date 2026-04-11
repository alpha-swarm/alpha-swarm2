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

function RunRow({ run }: { run: AgentRun }) {
  const preview = run.task_description.slice(0, TASK_PREVIEW_LENGTH);
  const truncated = run.task_description.length > TASK_PREVIEW_LENGTH;

  return (
    <TableRow>
      <TableCell><StatusBadge status={run.status} /></TableCell>
      <TableCell>
        <Link to={`/run/${encodeURIComponent(run.id ?? "")}`} className="text-primary hover:underline">
          {preview}{truncated ? "..." : ""}
        </Link>
        {run.progress_message && (
          <p className="text-xs text-muted-foreground mt-0.5 truncate max-w-md">
            {run.progress_message}
          </p>
        )}
      </TableCell>
      <TableCell className="text-xs font-mono">{run.model_used}</TableCell>
      <TableCell className="text-xs">
        {run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "-"}
      </TableCell>
      <TableCell className="text-xs">
        {run.quality_gate_passed === true ? "pass" : run.quality_gate_passed === false ? "fail" : "-"}
      </TableCell>
    </TableRow>
  );
}

export function RunsPage() {
  const { project } = useParams<{ project: string }>();
  const { runs, loading, error } = useRuns(project ?? "");
  const { metrics } = useProjectMetrics(project ?? "");
  const navigate = useNavigate();

  if (!project) return <p className="text-muted-foreground">No project selected</p>;
  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold">{project}</h1>
          <p className="text-sm text-muted-foreground">{runs.length} runs</p>
        </div>
        <Button onClick={() => navigate("/submit")}>New Task</Button>
      </div>

      {metrics && (
        <div className="mb-6">
          <MetricsGrid items={buildMetricItems(metrics)} compact />
        </div>
      )}

      {runs.length === 0 ? (
        <p className="text-muted-foreground">No runs yet.</p>
      ) : (
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
          <TableBody>
            {runs.map((run) => <RunRow key={run.id} run={run} />)}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
