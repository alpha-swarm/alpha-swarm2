import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useProjects } from "@/hooks/useProjects";
import { submitTask, createPlan } from "@/lib/mcp";

export function SubmitPage() {
  const { projects } = useProjects();
  const [project, setProject] = useState("");
  const [goal, setGoal] = useState("");
  const [planOnly, setPlanOnly] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!project || !goal) return;
    setSubmitting(true);
    setResult(null);
    try {
      const text = planOnly ? await createPlan(project, goal) : await submitTask(project, goal);
      setResult(text);
      setGoal("");
    } catch (err) {
      setResult(`Error: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="max-w-xl">
      <h1 className="text-2xl font-bold mb-6">Submit Task</h1>
      <Card>
        <CardHeader>
          <CardTitle className="text-base">New Agent Task</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="text-sm font-medium mb-1.5 block">Project</label>
              <select
                value={project}
                onChange={(e) => setProject(e.target.value)}
                className="w-full rounded-md border bg-background px-3 py-2 text-sm"
              >
                <option value="">Select project...</option>
                {projects.map((p) => (
                  <option key={p.name} value={p.name}>{p.name}</option>
                ))}
              </select>
            </div>
            <div>
              <label className="text-sm font-medium mb-1.5 block">Goal</label>
              <textarea
                value={goal}
                onChange={(e) => setGoal(e.target.value)}
                rows={5}
                placeholder="Describe what the agents should accomplish..."
                className="w-full rounded-md border bg-background px-3 py-2 text-sm resize-y font-mono"
              />
            </div>
            <div className="flex items-center gap-2">
              <input type="checkbox" id="plan-only" checked={planOnly} onChange={(e) => setPlanOnly(e.target.checked)} />
              <label htmlFor="plan-only" className="text-sm">Plan only (review before execution)</label>
            </div>
            <Button type="submit" disabled={submitting || !project || !goal}>
              {submitting ? "Submitting..." : planOnly ? "Create Plan" : "Submit Task"}
            </Button>
          </form>
        </CardContent>
      </Card>

      {result && (
        <Card className="mt-4">
          <CardContent className="pt-4">
            <pre className="text-xs whitespace-pre-wrap font-mono">{result}</pre>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
