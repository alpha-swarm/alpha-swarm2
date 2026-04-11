import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface MetricItem {
  label: string;
  value: number | string;
  className?: string;
}

interface Props {
  items: MetricItem[];
  columns?: string;
  compact?: boolean;
}

const COMPACT = { hdr: "pb-1 pt-3 px-3", lbl: "text-[10px]", val: "text-xl", cnt: "px-3 pb-3" };
const NORMAL = { hdr: "pb-2", lbl: "text-xs", val: "text-2xl", cnt: "" };

function MetricCard({ item, compact }: { item: MetricItem; compact: boolean }) {
  const s = compact ? COMPACT : NORMAL;
  return (
    <Card>
      <CardHeader className={s.hdr}>
        <CardTitle className={`${s.lbl} text-muted-foreground font-normal`}>{item.label}</CardTitle>
      </CardHeader>
      <CardContent className={s.cnt}>
        <span className={`${s.val} font-bold ${item.className ?? ""}`}>{item.value ?? 0}</span>
      </CardContent>
    </Card>
  );
}

export function MetricsGrid({ items, columns = "grid-cols-5", compact }: Props) {
  return (
    <div className={`grid ${columns} gap-3`}>
      {items.map((i) => <MetricCard key={i.label} item={i} compact={!!compact} />)}
    </div>
  );
}
