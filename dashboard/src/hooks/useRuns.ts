import { useState, useEffect } from "react";
import { resources } from "../lib/mcp";
import type { AgentRun } from "../types/swarm";

const REFRESH_INTERVAL_MS = 5_000;

export function useRuns(project: string) {
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!project) return;

    const fetch = async () => {
      try {
        const data = await resources.projectRuns(project);
        const parsed = Array.isArray(data) ? data : data?.[0]?.result ?? [];
        setRuns(parsed);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    };

    fetch();
    const interval = setInterval(fetch, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [project]);

  return { runs, loading, error };
}
