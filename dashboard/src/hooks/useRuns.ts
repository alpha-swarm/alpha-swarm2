import { useState, useEffect } from "react";
import { resources } from "@/lib/mcp";
import type { AgentRun } from "@/types/swarm";

const REFRESH_INTERVAL_MS = 5_000;

export function useRuns(project: string) {
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!project) return;
    const fetchRuns = async () => {
      try {
        setRuns(await resources.projectRuns(project));
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    };
    fetchRuns();
    const interval = setInterval(fetchRuns, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [project]);

  return { runs, loading, error };
}
