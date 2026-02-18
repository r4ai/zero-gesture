import type { Meta, StoryObj } from "@storybook/react"
import { Search } from "lucide-react"
import { TextField } from "./textfield"

const meta: Meta<typeof TextField> = {
  title: "UI/TextField",
  component: TextField,
  tags: ["autodocs"],
  argTypes: {
    label: { control: "text" },
    description: { control: "text" },
    errorMessage: { control: "text" },
    placeholder: { control: "text" },
    isDisabled: { control: "boolean" },
  },
}

export default meta
type Story = StoryObj<typeof TextField>

export const Default: Story = {
  args: {
    label: "Email",
    placeholder: "Enter your email",
  },
}

export const WithDescription: Story = {
  args: {
    label: "Email",
    placeholder: "Enter your email",
    description: "We'll never share your email with anyone else.",
  },
}

export const WithError: Story = {
  args: {
    label: "Email",
    placeholder: "Enter your email",
    errorMessage: "Please enter a valid email address.",
    isInvalid: true,
  },
}

export const Disabled: Story = {
  args: {
    label: "Email",
    placeholder: "Enter your email",
    isDisabled: true,
  },
}

export const WithIcon: Story = {
  render: (args) => (
    <TextField {...args}>
      <TextField.Icon slot="start">
        <Search />
      </TextField.Icon>
    </TextField>
  ),
  args: {
    placeholder: "Search...",
    "aria-label": "Search",
  },
}
