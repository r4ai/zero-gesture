import type { Meta, StoryObj } from "@storybook/react"
import { Badge } from "./badge"

const meta: Meta<typeof Badge> = {
  title: "UI/Badge",
  component: Badge,
  tags: ["autodocs"],
  argTypes: {
    variant: {
      control: "select",
      options: ["default", "outline", "key", "fallback", "success"],
    },
  },
}

export default meta
type Story = StoryObj<typeof Badge>

export const Default: Story = {
  args: {
    children: "Badge",
  },
}

export const Outline: Story = {
  args: {
    variant: "outline",
    children: "Outline",
  },
}

export const Key: Story = {
  args: {
    variant: "key",
    children: "Ctrl",
  },
}

export const Fallback: Story = {
  args: {
    variant: "fallback",
    children: "Fallback",
  },
}

export const Success: Story = {
  args: {
    variant: "success",
    children: "Success",
  },
}
