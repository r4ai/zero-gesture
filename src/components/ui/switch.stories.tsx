import type { Meta, StoryObj } from "@storybook/react"
import { Switch } from "./switch"

const meta: Meta<typeof Switch> = {
  title: "UI/Switch",
  component: Switch,
  tags: ["autodocs"],
  argTypes: {
    isDisabled: { control: "boolean" },
  },
}

export default meta
type Story = StoryObj<typeof Switch>

export const Default: Story = {
  args: {
    children: "Airplane Mode",
  },
}

export const Checked: Story = {
  args: {
    defaultSelected: true,
    children: "Bluetooth",
  },
}

export const Disabled: Story = {
  args: {
    isDisabled: true,
    children: "WiFi",
  },
}
