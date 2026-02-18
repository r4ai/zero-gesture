import type { Meta, StoryObj } from "@storybook/react"
import { useState } from "react"
import { TabContent, TabItem, TabList, Tabs } from "./tabs"

const meta: Meta<typeof Tabs> = {
  title: "UI/Tabs",
  component: Tabs,
  tags: ["autodocs"],
}

export default meta
type Story = StoryObj<typeof Tabs>

const TabsWithState = () => {
  const [selectedKey, setSelectedKey] = useState("general")

  return (
    <Tabs
      selectedKey={selectedKey}
      onSelectionChange={(key) => setSelectedKey(key as string)}
    >
      <TabList>
        <TabItem id="general">General</TabItem>
        <TabItem id="appearance">Appearance</TabItem>
        <TabItem id="advanced">Advanced</TabItem>
      </TabList>
      <TabContent id="general">
        <div className="p-4">General settings content</div>
      </TabContent>
      <TabContent id="appearance">
        <div className="p-4">Appearance settings content</div>
      </TabContent>
      <TabContent id="advanced">
        <div className="p-4">Advanced settings content</div>
      </TabContent>
    </Tabs>
  )
}

export const Default: Story = {
  render: () => <TabsWithState />,
}
