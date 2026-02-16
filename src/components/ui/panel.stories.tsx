import type { Meta, StoryObj } from "@storybook/react"
import { Button } from "./button"
import { Panel, PanelBody, PanelFooter, PanelHeader } from "./panel"

const meta: Meta<typeof Panel> = {
  title: "UI/Panel",
  component: Panel,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
}

export default meta
type Story = StoryObj<typeof Panel>

export const Default: Story = {
  render: () => (
    <div className="h-[400px] w-[600px] border">
      <Panel>
        <PanelHeader>
          <h2 className="font-semibold text-lg">Settings</h2>
        </PanelHeader>
        <PanelBody>
          <p className="text-muted-foreground">
            Manage your account settings and preferences here.
          </p>
          <div className="mt-4 h-96 space-y-4">
            <div className="h-20 rounded bg-muted/20" />
            <div className="h-20 rounded bg-muted/20" />
            <div className="h-20 rounded bg-muted/20" />
          </div>
        </PanelBody>
        <PanelFooter>
          <Button variant="outline" className="mr-2">
            Cancel
          </Button>
          <Button>Save Changes</Button>
        </PanelFooter>
      </Panel>
    </div>
  ),
}
