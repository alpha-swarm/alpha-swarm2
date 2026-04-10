import { Link } from "react-router";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { useProjects } from "@/hooks/useProjects";

export function ProjectsPage() {
  const { projects, loading, error } = useProjects();

  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;

  return (
    <div>
      <h1 className="text-2xl font-bold mb-6">Projects</h1>
      {projects.length === 0 ? (
        <p className="text-muted-foreground">
          No projects. <Link to="/submit" className="text-primary underline">Submit a task</Link> to get started.
        </p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Repository</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {projects.map((p) => (
              <TableRow key={p.name}>
                <TableCell className="font-medium">
                  <Link to={`/runs/${p.name}`} className="text-primary hover:underline">
                    {p.name}
                  </Link>
                </TableCell>
                <TableCell className="text-xs text-muted-foreground font-mono">
                  {p.repo_url}
                </TableCell>
                <TableCell>
                  <Badge variant="secondary">{p.status}</Badge>
                </TableCell>
                <TableCell>
                  <Link to={`/runs/${p.name}`} className="text-sm text-primary hover:underline">
                    View runs
                  </Link>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
