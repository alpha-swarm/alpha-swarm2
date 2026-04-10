import { useState, useEffect } from "react";
import { resources } from "../lib/mcp";
import type { AgentRun } from "../types/swarm";

const REFRESH_INTERVAL_MS = 3_000;

export function useRunDetail(runId: string) {
  const [run, setRun] = useState<AgentRun | null>(null);
  const [subRuns, setSubRuns] = useState<AgentRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!runId) return;

    const fetch = async () => {
      try {
        const [runData, subData] = await Promise.all([
          resources.runDetail(runId),
          resources.subRuns(runId),
        ]);

        const runParsed = Array.isArray(runData) ? runData[0]?.result?.[0] : runData?.[0]?.result?.[0];
        const subParsed = Array.isArray(subData) ? subData : subData?.[0]?.result ?? [];

        setRun(runParsed ?? null);
        setSubRuns(subParsed);
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
  }, [runId]);

  return { run, subRuns, loading, error };
}
