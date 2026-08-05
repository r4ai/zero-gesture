import type { Meta, StoryObj } from "@storybook/react"
import { expect, fn, userEvent, within } from "storybook/test"
import { Panel, PanelBody, PanelHeader } from "@/components/ui/panel"
import { ConfigDraftContextProvider } from "@/contexts/config-draft"
import { DEFAULTS } from "@/types/config"
import { SettingsFormActions } from "./settings-form-actions"

type StoryArgs = {
  isDirty: boolean
  isSaving: boolean
  saveError: string | null
  onSave: () => void
  onCancel: () => void
}

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
    onSave: { action: "save" },
    onCancel: { action: "cancel" },
  },
  decorators: [
    (Story, { args }) => (
      <ConfigDraftContextProvider
        value={{
          draft: DEFAULTS,
          setDraft: () => {},
          isDirty: args.isDirty,
          reset: args.onCancel,
          save: args.onSave,
          adoptApplied: () => {},
          isSaving: args.isSaving,
          saveError: args.saveError,
        }}
      >
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
      </ConfigDraftContextProvider>
    ),
  ],
  args: {
    isDirty: false,
    isSaving: false,
    saveError: null,
    onSave: fn(),
    onCancel: fn(),
  },
} satisfies Meta<StoryArgs>

export default meta
type Story = StoryObj<typeof meta>

/**
 * Default state with no changes (buttons are disabled)
 */
export const Default: Story = {}

/**
 * State with unsaved changes (buttons are enabled)
 */
export const WithChanges: Story = {
  args: {
    isDirty: true,
  },
}

/**
 * State while saving (buttons show loading state)
 */
export const Saving: Story = {
  args: {
    isDirty: true,
    isSaving: true,
  },
}

export const RetryableFailure: Story = {
  args: {
    isDirty: true,
    saveError:
      "Save was not applied because newer settings are available. Your draft was kept; review it and retry.",
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole("alert")).toHaveTextContent("draft was kept")
    await userEvent.click(canvas.getByRole("button", { name: "Save Changes" }))
    await expect(args.onSave).toHaveBeenCalledOnce()
  },
}
