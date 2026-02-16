import type { Meta, StoryObj } from "@storybook/react"
import { Panel, PanelBody, PanelHeader } from "@/components/ui/panel"
import {
  SettingsFormActions,
  type SettingsFormActionsProps,
} from "./settings-form-actions"

const meta = {
  title: "Components/SettingsFormActions",
  component: SettingsFormActions,
  parameters: {
    layout: "centered",
  },
  tags: ["autodocs"],
  argTypes: {
    isDirty: {
      control: "boolean",
      description: "Whether there are unsaved changes",
    },
    isSaving: {
      control: "boolean",
      description: "Whether the form is currently saving",
    },
  },
  decorators: [
    (Story) => (
      <div className="w-[600px]">
        <Panel>
          <PanelHeader>
            <div className="flex flex-col gap-0.5">
              <h2 className="font-semibold text-[18px]">Settings Example</h2>
              <p className="text-[12px] text-foreground-subtle">
                Example settings page with form actions
              </p>
            </div>
          </PanelHeader>
          <PanelBody>
            <div className="rounded-[10px] border border-border bg-background-elevated p-5">
              <p className="text-[14px]">
                Some settings content would go here...
              </p>
            </div>
          </PanelBody>
          <Story />
        </Panel>
      </div>
    ),
  ],
} satisfies Meta<SettingsFormActionsProps>

export default meta
type Story = StoryObj<typeof meta>

/**
 * Default state with no changes (buttons are disabled)
 */
export const Default: Story = {
  args: {
    isDirty: false,
    isSaving: false,
    onSave: () => {},
    onCancel: () => {},
  },
}

/**
 * State with unsaved changes (buttons are enabled)
 */
export const WithChanges: Story = {
  args: {
    isDirty: true,
    isSaving: false,
    onSave: () => {},
    onCancel: () => {},
  },
}

/**
 * State while saving (buttons show loading state)
 */
export const Saving: Story = {
  args: {
    isDirty: true,
    isSaving: true,
    onSave: () => {},
    onCancel: () => {},
  },
}
