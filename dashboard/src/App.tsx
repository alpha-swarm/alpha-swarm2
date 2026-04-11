import { Routes, Route } from "react-router";
import { Layout } from "@/components/Layout";
import { DashboardPage } from "@/pages/Dashboard";
import { RunDetailPage } from "@/pages/RunDetail";
import { SubmitPage } from "@/pages/Submit";

export function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route index element={<DashboardPage />} />
        <Route path="run/:id" element={<RunDetailPage />} />
        <Route path="submit" element={<SubmitPage />} />
      </Route>
    </Routes>
  );
}
