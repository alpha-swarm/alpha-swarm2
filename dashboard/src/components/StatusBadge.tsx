import { Badge } from "@/components/ui/badge";
import type { RunStatus } from "@/types/swarm";

const STATUS_VARIANT: Record<RunStatus, "default" | "secondary" | "destructive" | "outline"> = {
  pending: "secondary",
  planning: "outline",
  planned: "outline",
  approved: "default",
  running: "default",
  passed: "default",
  failed: "destructive",
  skipped: "secondary",
};

export function StatusBadge({ status }: { status: RunStatus }) {
  return (
    <Badge variant={STATUS_VARIANT[status] ?? "secondary"}>
      {status}
    </Badge>
  );
}
