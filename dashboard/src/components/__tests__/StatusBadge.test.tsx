import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { StatusBadge } from "../StatusBadge";

describe("StatusBadge", () => {
  it("renders the status text", () => {
    render(<StatusBadge status="running" />);
    expect(screen.getByText("running")).toBeInTheDocument();
  });

  it("renders all status variants without error", () => {
    const statuses = ["pending", "planning", "planned", "approved", "running", "passed", "failed", "skipped"] as const;
    for (const status of statuses) {
      const { unmount } = render(<StatusBadge status={status} />);
      expect(screen.getByText(status)).toBeInTheDocument();
      unmount();
    }
  });
});
