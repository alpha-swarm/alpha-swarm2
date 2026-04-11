import { Routes, Route } from "react-router";
import { Layout } from "@/components/Layout";
import { DashboardPage } from "@/pages/Dashboard";
import { RunDetailPage } from "@/pages/RunDetail";

export function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route index element={<DashboardPage />} />
        <Route path="run/:id" element={<RunDetailPage />} />
      </Route>
    </Routes>
  );
}
