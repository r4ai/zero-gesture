import type { Meta, StoryObj } from "@storybook/react"
import { type ComponentProps, useState } from "react"
import { ComboBox, ComboBoxItem } from "./combobox"

const items = ["Apple", "Banana", "Orange"]

const meta: Meta<typeof ComboBox> = {
  title: "UI/ComboBox",
  component: ComboBox,
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
type Story = StoryObj<typeof ComboBox>

function ComboBoxWithState(
  props: Omit<ComponentProps<typeof ComboBox>, "children">,
) {
  const [selectedKey, setSelectedKey] = useState<string | null>(null)

  return (
    <ComboBox
      selectedKey={selectedKey}
      onSelectionChange={(key) => setSelectedKey(key ? String(key) : null)}
      {...props}
    >
      {items.map((item) => (
        <ComboBoxItem key={item} id={item} textValue={item}>
          {item}
        </ComboBoxItem>
      ))}
    </ComboBox>
  )
}

export const Default: Story = {
  render: () => (
    <ComboBoxWithState label="Favorite Fruit" placeholder="Select a fruit" />
  ),
}

export const WithDescription: Story = {
  render: () => (
    <ComboBoxWithState
      label="Favorite Fruit"
      placeholder="Select a fruit"
      description="Type to filter, then choose from the list."
    />
  ),
}

export const Disabled: Story = {
  render: () => (
    <ComboBoxWithState
      label="Favorite Fruit"
      placeholder="Select a fruit"
      isDisabled
    />
  ),
}
