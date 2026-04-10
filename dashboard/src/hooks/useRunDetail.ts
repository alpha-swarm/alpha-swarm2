import { useState, useEffect } from "react";
import { resources } from "@/lib/mcp";
import type { AgentRun } from "@/types/swarm";

const REFRESH_INTERVAL_MS = 3_000;

export function useRunDetail(runId: string) {
  const [run, setRun] = useState<AgentRun | null>(null);
  const [subRuns, setSubRuns] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!runId) return;
    const fetchDetail = async () => {
      try {
        const [runData, subData] = await Promise.all([
          resources.runDetail(runId),
          resources.subRuns(runId),
        ]);
        setRun(runData[0] ?? null);
        setSubRuns(subData);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    };
    fetchDetail();
    const interval = setInterval(fetchDetail, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [runId]);

  return { run, subRuns, loading, error };
}
