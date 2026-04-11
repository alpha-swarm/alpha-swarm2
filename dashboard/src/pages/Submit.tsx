import { useState } from "react";
import { useNavigate } from "react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useProjects } from "@/hooks/useProjects";
import { submitTask, createPlan, approvePlan, planFeedback, getRunStatus } from "@/lib/mcp";
import type { Project } from "@/types/swarm";

const RUN_ID_PATTERN = /Run ID:\s*(\S+)/;

function formatError(err: unknown): string {
  return `Error: ${err instanceof Error ? err.message : String(err)}`;
}

interface SubmitFormProps {
  projects: Project[];
  submitting: boolean;
  onSubmit: (project: string, goal: string, planOnly: boolean) => void;
}

function ProjectSelect({ project, projects, onChange }: { project: string; projects: Project[]; onChange: (v: string) => void }) {
  return (
    <div>
      <label className="text-sm font-medium mb-1.5 block">Project</label>
      <select value={project} onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-md border bg-background px-3 py-2 text-sm">
        <option value="">Select project...</option>
        {projects.map((p) => (
          <option key={p.name} value={p.name}>{p.name}</option>
        ))}
      </select>
    </div>
  );
}

function GoalTextarea({ goal, onChange }: { goal: string; onChange: (v: string) => void }) {
  return (
    <div>
      <label className="text-sm font-medium mb-1.5 block">Goal</label>
      <textarea value={goal} onChange={(e) => onChange(e.target.value)} rows={5}
        placeholder="Describe what the agents should accomplish..."
        className="w-full rounded-md border bg-background px-3 py-2 text-sm resize-y font-mono" />
    </div>
  );
}

function submitButtonLabel(submitting: boolean, planOnly: boolean): string {
  if (submitting) return "Submitting...";
  return planOnly ? "Create Plan" : "Submit Task";
}

function SubmitForm({ projects, submitting, onSubmit }: SubmitFormProps) {
  const [project, setProject] = useState("");
  const [goal, setGoal] = useState("");
  const [planOnly, setPlanOnly] = useState(false);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!project || !goal) return;
    onSubmit(project, goal, planOnly);
    if (!planOnly) setGoal("");
  };

  return (
    <Card>
      <CardHeader><CardTitle className="text-base">New Agent Task</CardTitle></CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-4">
          <ProjectSelect project={project} projects={projects} onChange={setProject} />
          <GoalTextarea goal={goal} onChange={setGoal} />
          <div className="flex items-center gap-2">
            <input type="checkbox" id="plan-only" checked={planOnly} onChange={(e) => setPlanOnly(e.target.checked)} />
            <label htmlFor="plan-only" className="text-sm">Plan only (review before execution)</label>
          </div>
          <Button type="submit" disabled={submitting || !project || !goal}>
            {submitButtonLabel(submitting, planOnly)}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

interface SubmitResultProps {
  result: string;
  runId: string | null;
  planOnly: boolean;
  submitting: boolean;
  onCheckStatus: () => void;
  onApprove: () => void;
  onFeedback: (feedback: string) => void;
}

function SubmitResult({ result, runId, planOnly, submitting, onCheckStatus, onApprove, onFeedback }: SubmitResultProps) {
  const navigate = useNavigate();
  const [feedback, setFeedback] = useState("");

  const handleFeedback = () => {
    if (!feedback) return;
    onFeedback(feedback);
    setFeedback("");
  };

  return (
    <Card className="mt-4">
      <CardContent className="pt-4">
        <pre className="text-xs whitespace-pre-wrap font-mono mb-4">{result}</pre>
        {runId && (
          <div className="flex gap-2 flex-wrap">
            <Button variant="outline" size="sm" onClick={onCheckStatus}>Check Status</Button>
            <Button variant="outline" size="sm" onClick={() => navigate(`/run/${encodeURIComponent(runId)}`)}>View Detail</Button>
            {planOnly && (
              <>
                <Button size="sm" onClick={onApprove} disabled={submitting}>Approve Plan</Button>
                <div className="flex gap-2 w-full mt-2">
                  <input value={feedback} onChange={(e) => setFeedback(e.target.value)}
                    placeholder="Feedback for re-planning..."
                    className="flex-1 rounded-md border bg-background px-3 py-1 text-sm" />
                  <Button variant="secondary" size="sm" onClick={handleFeedback} disabled={submitting || !feedback}>
                    Send Feedback
                  </Button>
                </div>
              </>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

async function mcpAction(fn: () => Promise<string>, set: (s: string) => void, setBusy: (b: boolean) => void) {
  setBusy(true);
  try { set(await fn()); }
  catch (err) { set(formatError(err)); }
  finally { setBusy(false); }
}

function useSubmitActions() {
  const [result, setResult] = useState<string | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [lastPlanOnly, setLastPlanOnly] = useState(false);

  const handleSubmit = async (project: string, goal: string, planOnly: boolean) => {
    setResult(null); setRunId(null); setLastPlanOnly(planOnly);
    await mcpAction(
      () => planOnly ? createPlan(project, goal) : submitTask(project, goal),
      (text) => { setResult(text); const m = text.match(RUN_ID_PATTERN); if (m) setRunId(m[1].replace(/\.$/, "")); },
      setSubmitting,
    );
  };

  const handleApprove = async () => { if (runId) await mcpAction(() => approvePlan(runId), setResult, setSubmitting); };
  const handleFeedback = async (fb: string) => { if (runId) await mcpAction(() => planFeedback(runId, fb), setResult, setSubmitting); };
  const handleCheckStatus = async () => { if (runId) try { setResult(await getRunStatus(runId)); } catch (e) { setResult(formatError(e)); } };

  return { result, runId, submitting, lastPlanOnly, handleSubmit, handleApprove, handleFeedback, handleCheckStatus };
}

export function SubmitPage() {
  const { projects } = useProjects();
  const actions = useSubmitActions();

  return (
    <div className="max-w-xl">
      <h1 className="text-2xl font-bold mb-6">Submit Task</h1>
      <SubmitForm projects={projects} submitting={actions.submitting} onSubmit={actions.handleSubmit} />
      {actions.result && (
        <SubmitResult
          result={actions.result}
          runId={actions.runId}
          planOnly={actions.lastPlanOnly}
          submitting={actions.submitting}
          onCheckStatus={actions.handleCheckStatus}
          onApprove={actions.handleApprove}
          onFeedback={actions.handleFeedback}
        />
      )}
    </div>
  );
}
