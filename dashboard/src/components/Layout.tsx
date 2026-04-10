import { NavLink, Outlet } from "react-router";
import { cn } from "@/lib/utils";

const NAV_ITEMS = [
  { to: "/", label: "Dashboard" },
  { to: "/projects", label: "Projects" },
  { to: "/submit", label: "Submit Task" },
];

export function Layout() {
  return (
    <div className="flex min-h-screen">
      <nav className="w-52 border-r bg-sidebar p-4">
        <h2 className="mb-6 text-sm font-bold tracking-tight text-primary">
          alpha-swarm
        </h2>
        {NAV_ITEMS.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === "/"}
            className={({ isActive }) =>
              cn(
                "block rounded-md px-3 py-2 text-sm mb-1 transition-colors",
                isActive
                  ? "bg-sidebar-accent text-sidebar-accent-foreground font-semibold"
                  : "text-muted-foreground hover:bg-sidebar-accent/50"
              )
            }
          >
            {item.label}
          </NavLink>
        ))}
      </nav>
      <main className="flex-1 overflow-auto p-6">
        <Outlet />
      </main>
    </div>
  );
}
