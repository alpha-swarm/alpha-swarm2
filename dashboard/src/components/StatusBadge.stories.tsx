import type { Meta, StoryObj } from "@storybook/react";
import { StatusBadge } from "./StatusBadge";
import type { RunStatus } from "@/types/swarm";

const meta: Meta<typeof StatusBadge> = {
  component: StatusBadge,
  title: "Components/StatusBadge",
};
export default meta;

type Story = StoryObj<typeof StatusBadge>;

const statuses: RunStatus[] = ["pending", "planning", "planned", "approved", "running", "passed", "failed", "skipped"];

export const AllStatuses: Story = {
  render: () => (
    <div className="flex gap-2 flex-wrap">
      {statuses.map((s) => <StatusBadge key={s} status={s} />)}
    </div>
  ),
};

export const Passed: Story = { args: { status: "passed" } };
export const Failed: Story = { args: { status: "failed" } };
export const Running: Story = { args: { status: "running" } };
