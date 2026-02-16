import type { Meta, StoryObj } from "@storybook/react"
import { Select, SelectItem } from "./select"

const meta: Meta<typeof Select> = {
  title: "UI/Select",
  component: Select,
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
type Story = StoryObj<typeof Select>

export const Default: Story = {
  args: {
    label: "Favorite Fruit",
    placeholder: "Select a fruit",
    children: [
      <SelectItem key="apple" id="apple">
        Apple
      </SelectItem>,
      <SelectItem key="banana" id="banana">
        Banana
      </SelectItem>,
      <SelectItem key="orange" id="orange">
        Orange
      </SelectItem>,
    ],
  },
}

export const WithDescription: Story = {
  args: {
    label: "Favorite Fruit",
    placeholder: "Select a fruit",
    description: "Select your favorite fruit from the list.",
    children: [
      <SelectItem key="apple" id="apple">
        Apple
      </SelectItem>,
      <SelectItem key="banana" id="banana">
        Banana
      </SelectItem>,
      <SelectItem key="orange" id="orange">
        Orange
      </SelectItem>,
    ],
  },
}

export const Disabled: Story = {
  args: {
    label: "Favorite Fruit",
    placeholder: "Select a fruit",
    isDisabled: true,
    children: [
      <SelectItem key="apple" id="apple">
        Apple
      </SelectItem>,
      <SelectItem key="banana" id="banana">
        Banana
      </SelectItem>,
      <SelectItem key="orange" id="orange">
        Orange
      </SelectItem>,
    ],
  },
}
