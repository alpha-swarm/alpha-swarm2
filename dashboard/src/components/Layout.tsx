import { Outlet } from "react-router";
import { TopBar } from "@/components/TopBar";
import { useEventStream } from "@/hooks/useEventStream";

export function Layout() {
  const { connected } = useEventStream();
  return (
    <div className="min-h-screen bg-background">
      <TopBar connected={connected} />
      <main className="max-w-5xl mx-auto px-4 py-4">
        <Outlet />
      </main>
    </div>
  );
}
