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
  props: Omit<React.ComponentProps<typeof KeyInput>, "sequence" | "onChange">,
) => {
  const [sequence, setSequence] = useState<string[][]>([])

  return <KeyInput sequence={sequence} onChange={setSequence} {...props} />
}

export const Default: Story = {
  render: () => <KeyInputWithState />,
}

export const WithKeys: Story = {
  render: () => {
    const [sequence, setSequence] = useState<string[][]>([
      ["ctrl", "shift", "a"],
      ["f21", "a"],
    ])
    return <KeyInput sequence={sequence} onChange={setSequence} />
  },
}

export const Disabled: Story = {
  render: () => {
    const [sequence, setSequence] = useState<string[][]>([["ctrl", "c"]])
    return <KeyInput sequence={sequence} onChange={setSequence} isDisabled />
  },
}

export const CustomPlaceholder: Story = {
  render: () => <KeyInputWithState placeholder="Press keys..." />,
}

export const CustomHint: Story = {
  render: () => <KeyInputWithState hint="Custom hint message" />,
}
