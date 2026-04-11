import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface MetricItem {
  label: string;
  value: number | string;
  className?: string;
}

interface MetricsGridProps {
  items: MetricItem[];
  columns?: string;
  compact?: boolean;
}

export function MetricsGrid({ items, columns = "grid-cols-5", compact }: MetricsGridProps) {
  return (
    <div className={`grid ${columns} gap-3`}>
      {items.map((item) => (
        <Card key={item.label}>
          <CardHeader className={compact ? "pb-1 pt-3 px-3" : "pb-2"}>
            <CardTitle className={`${compact ? "text-[10px]" : "text-xs"} text-muted-foreground font-normal`}>
              {item.label}
            </CardTitle>
          </CardHeader>
          <CardContent className={compact ? "px-3 pb-3" : ""}>
            <span className={`${compact ? "text-xl" : "text-2xl"} font-bold ${item.className ?? ""}`}>
              {item.value ?? 0}
            </span>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
