import { useState, useEffect, useCallback } from "react";
import { resources } from "@/lib/mcp";
import type { Project } from "@/types/swarm";

const REFRESH_INTERVAL_MS = 10_000;

export function useProjects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async () => {
    try {
      setProjects(await resources.projects());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refetch();
    const interval = setInterval(refetch, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [refetch]);

  return { projects, loading, error, refetch };
}
