import { useParams, Link } from "react-router";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { StatusBadge } from "@/components/StatusBadge";
import { useRuns } from "@/hooks/useRuns";

export function RunsPage() {
  const { project } = useParams<{ project: string }>();
  const { runs, loading, error } = useRuns(project ?? "");

  if (!project) return <p className="text-muted-foreground">No project selected</p>;
  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;

  return (
    <div>
      <h1 className="text-2xl font-bold mb-1">{project}</h1>
      <p className="text-sm text-muted-foreground mb-6">{runs.length} runs</p>
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
          {runs.map((run) => (
            <TableRow key={run.id}>
              <TableCell><StatusBadge status={run.status} /></TableCell>
              <TableCell>
                <Link to={`/run/${encodeURIComponent(run.id ?? "")}`} className="text-primary hover:underline">
                  {run.task_description.slice(0, 80)}
                  {run.task_description.length > 80 ? "..." : ""}
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
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
