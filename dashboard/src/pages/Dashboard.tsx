import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useDashboard } from "@/hooks/useDashboard";

export function DashboardPage() {
  const { stats, loading, error } = useDashboard();

  if (loading) return <p className="text-muted-foreground">Loading...</p>;
  if (error) return <p className="text-destructive">Error: {error}</p>;
  if (!stats) return <p className="text-muted-foreground">No data</p>;

  const cards = [
    { label: "Active", value: stats.active, className: "text-yellow-500" },
    { label: "Passed", value: stats.passed, className: "text-green-500" },
    { label: "Failed", value: stats.failed, className: "text-destructive" },
    { label: "Pending", value: stats.pending, className: "text-muted-foreground" },
    { label: "Total Runs", value: stats.total_runs, className: "" },
  ];

  return (
    <div>
      <h1 className="text-2xl font-bold mb-6">Dashboard</h1>
      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4">
        {cards.map((card) => (
          <Card key={card.label}>
            <CardHeader className="pb-2">
              <CardTitle className="text-xs text-muted-foreground font-normal">
                {card.label}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <div className={`text-2xl font-bold ${card.className}`}>
                {card.value}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}
