import { useState } from "react";
import { Link } from "react-router";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useProjects } from "@/hooks/useProjects";
import { createProject, deleteProject } from "@/lib/mcp";

export function ProjectsPage() {
  const { projects, loading, error, refetch } = useProjects();
  const [showForm, setShowForm] = useState(false);
  const [name, setName] = useState("");
  const [repoUrl, setRepoUrl] = useState("");
  const [desc, setDesc] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name || !repoUrl) return;
    setSubmitting(true);
    try {
      await createProject(name, repoUrl, desc || undefined);
      setName(""); setRepoUrl(""); setDesc(""); setShowForm(false);
      refetch();
    } catch { /* toast later */ }
    finally { setSubmitting(false); }
  };

  const handleDelete = async (projectName: string) => {
    if (!confirm(`Delete project "${projectName}" and all its data?`)) return;
    await deleteProject(projectName);
    refetch();
  };

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

      {showForm && (
        <Card className="mb-6">
          <CardHeader><CardTitle className="text-base">Create Project</CardTitle></CardHeader>
          <CardContent>
            <form onSubmit={handleCreate} className="grid grid-cols-3 gap-3">
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
      )}

      {projects.length === 0 ? (
        <p className="text-muted-foreground">No projects yet.</p>
      ) : (
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
              <TableRow key={p.name}>
                <TableCell className="font-medium">
                  <Link to={`/runs/${p.name}`} className="text-primary hover:underline">{p.name}</Link>
                </TableCell>
                <TableCell className="text-xs text-muted-foreground font-mono truncate max-w-xs">
                  {p.repo_url}
                </TableCell>
                <TableCell><Badge variant="secondary">{p.status}</Badge></TableCell>
                <TableCell>
                  <div className="flex gap-2">
                    <Link to={`/runs/${p.name}`}><Button variant="outline" size="sm">Runs</Button></Link>
                    <Button variant="destructive" size="sm" onClick={() => handleDelete(p.name)}>Delete</Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
