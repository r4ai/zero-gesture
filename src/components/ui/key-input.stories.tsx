import type { Meta, StoryObj } from "@storybook/react"
import { useState } from "react"
import { KeyInput } from "./key-input"

const meta: Meta<typeof KeyInput> = {
  title: "UI/KeyInput",
  component: KeyInput,
  tags: ["autodocs"],
}

export default meta
type Story = StoryObj<typeof KeyInput>

const KeyInputWithState = (
  props: Omit<React.ComponentProps<typeof KeyInput>, "keys" | "onChange">,
) => {
  const [keys, setKeys] = useState<string[]>([])

  return <KeyInput keys={keys} onChange={setKeys} {...props} />
}

export const Default: Story = {
  render: () => <KeyInputWithState />,
}

export const WithKeys: Story = {
  render: () => {
    const [keys, setKeys] = useState<string[]>(["Ctrl", "Shift", "A"])
    return <KeyInput keys={keys} onChange={setKeys} />
  },
}

export const Disabled: Story = {
  render: () => {
    const [keys, setKeys] = useState<string[]>(["Ctrl", "C"])
    return <KeyInput keys={keys} onChange={setKeys} isDisabled />
  },
}

export const CustomPlaceholder: Story = {
  render: () => <KeyInputWithState placeholder="Press keys..." />,
}

export const CustomHint: Story = {
  render: () => <KeyInputWithState hint="Custom hint message" />,
}
