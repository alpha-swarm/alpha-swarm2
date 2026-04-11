import { useState, useEffect } from "react";
import { resources } from "@/lib/mcp";

export interface ProjectMetrics {
  total_runs: number;
  passed: number;
  failed: number;
  active: number;
  pending: number;
}

const REFRESH_INTERVAL_MS = 5_000;

export function useProjectMetrics(project: string) {
  const [metrics, setMetrics] = useState<ProjectMetrics | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!project) return;
    const fetchMetrics = async () => {
      try {
        const data = await resources.projectMetrics(project);
        setMetrics((data[0] as ProjectMetrics) ?? null);
      } catch {
        // Silently fail — metrics are supplementary
      } finally {
        setLoading(false);
      }
    };
    fetchMetrics();
    const interval = setInterval(fetchMetrics, REFRESH_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [project]);

  return { metrics, loading };
}
