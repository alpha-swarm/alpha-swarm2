import { useState } from "react";
import { Link } from "react-router";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useProjects } from "@/hooks/useProjects";
import { createProject, deleteProject } from "@/lib/mcp";
import type { Project } from "@/types/swarm";

interface CreateProjectFormProps {
  submitting: boolean;
  onCreate: (name: string, repoUrl: string, desc: string) => void;
}

function CreateProjectForm({ submitting, onCreate }: CreateProjectFormProps) {
  const [name, setName] = useState("");
  const [repoUrl, setRepoUrl] = useState("");
  const [desc, setDesc] = useState("");

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name || !repoUrl) return;
    onCreate(name, repoUrl, desc);
    setName("");
    setRepoUrl("");
    setDesc("");
  };

  return (
    <Card className="mb-6">
      <CardHeader><CardTitle className="text-base">Create Project</CardTitle></CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="grid grid-cols-3 gap-3">
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name"
            className="rounded-md border bg-background px-3 py-2 text-sm" />
          <input value={repoUrl} onChange={(e) => setRepoUrl(e.target.value)} placeholder="Repo URL or path"
            className="rounded-md border bg-background px-3 py-2 text-sm" />
          <input value={desc} onChange={(e) => setDesc(e.target.value)} placeholder="Description (optional)"
            className="rounded-md border bg-background px-3 py-2 text-sm" />
          <Button type="submit" disabled={submitting || !name || !repoUrl} className="col-span-3 w-fit">
            {submitting ? "Creating..." : "Create"}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function ProjectRow({ project, onDelete }: { project: Project; onDelete: (name: string) => void }) {
  return (
    <TableRow>
      <TableCell className="font-medium">
        <Link to={`/runs/${project.name}`} className="text-primary hover:underline">{project.name}</Link>
      </TableCell>
      <TableCell className="text-xs text-muted-foreground font-mono truncate max-w-xs">
        {project.repo_url}
      </TableCell>
      <TableCell><Badge variant="secondary">{project.status}</Badge></TableCell>
      <TableCell>
        <div className="flex gap-2">
          <Link to={`/runs/${project.name}`}><Button variant="outline" size="sm">Runs</Button></Link>
          <Button variant="destructive" size="sm" onClick={() => onDelete(project.name)}>Delete</Button>
        </div>
      </TableCell>
    </TableRow>
  );
}

function useProjectActions(refetch: () => void) {
  const [showForm, setShowForm] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const handleCreate = async (name: string, repoUrl: string, desc: string) => {
    setSubmitting(true);
    try {
      await createProject(name, repoUrl, desc || undefined);
      setShowForm(false);
      refetch();
    } catch { /* toast later */ }
    finally { setSubmitting(false); }
  };

  const handleDelete = async (projectName: string) => {
    if (!confirm(`Delete project "${projectName}" and all its data?`)) return;
    await deleteProject(projectName);
    refetch();
  };

  return { showForm, setShowForm, submitting, handleCreate, handleDelete };
}

function ProjectTable({ projects, onDelete }: { projects: Project[]; onDelete: (name: string) => void }) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Name</TableHead>
          <TableHead>Repository</TableHead>
          <TableHead>Status</TableHead>
          <TableHead className="w-32">Actions</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {projects.map((p) => (
          <ProjectRow key={p.name} project={p} onDelete={onDelete} />
        ))}
      </TableBody>
    </Table>
  );
}

export function ProjectsPage() {
  const { projects, loading, error, refetch } = useProjects();
  const { showForm, setShowForm, submitting, handleCreate, handleDelete } = useProjectActions(refetch);

  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Projects</h1>
        <Button onClick={() => setShowForm(!showForm)}>
          {showForm ? "Cancel" : "New Project"}
        </Button>
      </div>
      {showForm && <CreateProjectForm submitting={submitting} onCreate={handleCreate} />}
      {projects.length === 0
        ? <p className="text-muted-foreground">No projects yet.</p>
        : <ProjectTable projects={projects} onDelete={handleDelete} />}
    </div>
  );
}
