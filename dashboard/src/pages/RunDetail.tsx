import { useParams } from "react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Separator } from "@/components/ui/separator";
import { StatusBadge } from "@/components/StatusBadge";
import { useRunDetail } from "@/hooks/useRunDetail";

export function RunDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { run, subRuns, loading, error } = useRunDetail(id ?? "");

  if (!id) return <p className="text-muted-foreground">No run ID</p>;
  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;
  if (!run) return <p className="text-muted-foreground">Run not found</p>;

  return (
    <div className="space-y-6">
      <div>
        <StatusBadge status={run.status} />
        <h1 className="text-xl font-bold mt-2">{run.task_description.slice(0, 120)}</h1>
        <p className="text-xs text-muted-foreground mt-1 font-mono">
          {run.id} | {run.model_used} | {run.duration_ms > 0 ? `${(run.duration_ms / 1000).toFixed(0)}s` : "running..."}
        </p>
      </div>

      {run.progress_message && (
        <Card>
          <CardContent className="pt-4">
            <code className="text-xs whitespace-pre-wrap">{run.progress_message}</code>
          </CardContent>
        </Card>
      )}

      {run.error_message && (
        <Card className="border-destructive">
          <CardContent className="pt-4 text-destructive text-sm">
            {run.error_message}
          </CardContent>
        </Card>
      )}

      {run.phase_timings && (
        <div>
          <h3 className="text-sm font-semibold mb-3">Phase Timings</h3>
          <div className="grid grid-cols-5 gap-2">
            {Object.entries(run.phase_timings).map(([key, ms]) => (
              <Card key={key}>
                <CardHeader className="pb-1 pt-3 px-3">
                  <CardTitle className="text-[10px] text-muted-foreground font-normal">
                    {key.replace("_ms", "")}
                  </CardTitle>
                </CardHeader>
                <CardContent className="px-3 pb-3">
                  <span className="text-lg font-bold">{((ms as number) / 1000).toFixed(1)}s</span>
                </CardContent>
              </Card>
            ))}
          </div>
        </div>
      )}

      {subRuns.length > 0 && (
        <div>
          <h3 className="text-sm font-semibold mb-3">Sub-tasks ({subRuns.length})</h3>
          <div className="space-y-1">
            {subRuns.map((sub) => (
              <div key={sub.id} className="flex items-center gap-3 py-2 px-3 rounded-md hover:bg-muted/50">
                <StatusBadge status={sub.status} />
                <span className="text-sm truncate flex-1">{sub.task_description}</span>
                {sub.progress_message && (
                  <span className="text-[10px] text-muted-foreground truncate max-w-48">
                    {sub.progress_message}
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {run.tool_calls.length > 0 && (
        <div>
          <Separator className="mb-4" />
          <h3 className="text-sm font-semibold mb-3">Tool Calls ({run.tool_calls.length})</h3>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-28">Tool</TableHead>
                <TableHead>Params</TableHead>
                <TableHead>Result</TableHead>
                <TableHead className="w-16">Time</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {run.tool_calls.map((tc, i) => (
                <TableRow key={i} className={tc.is_error ? "text-destructive" : ""}>
                  <TableCell className="font-mono text-xs">{tc.tool}</TableCell>
                  <TableCell className="text-xs max-w-48 truncate">{tc.params_preview}</TableCell>
                  <TableCell className="text-xs max-w-64 truncate">{tc.result_preview}</TableCell>
                  <TableCell className="text-xs">{(tc.duration_ms / 1000).toFixed(1)}s</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}
