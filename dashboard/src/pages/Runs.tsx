import { useParams, Link, useNavigate } from "react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Button } from "@/components/ui/button";
import { StatusBadge } from "@/components/StatusBadge";
import { useRuns } from "@/hooks/useRuns";
import { useProjectMetrics } from "@/hooks/useProjectMetrics";

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
        <div className="grid grid-cols-5 gap-3 mb-6">
          {[
            { label: "Total", value: metrics.total_runs, className: "" },
            { label: "Active", value: metrics.active, className: "text-yellow-500" },
            { label: "Passed", value: metrics.passed, className: "text-green-500" },
            { label: "Failed", value: metrics.failed, className: "text-destructive" },
            { label: "Pending", value: metrics.pending, className: "text-muted-foreground" },
          ].map((c) => (
            <Card key={c.label}>
              <CardHeader className="pb-1 pt-3 px-3">
                <CardTitle className="text-[10px] text-muted-foreground font-normal">{c.label}</CardTitle>
              </CardHeader>
              <CardContent className="px-3 pb-3">
                <span className={`text-xl font-bold ${c.className}`}>{c.value ?? 0}</span>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

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
