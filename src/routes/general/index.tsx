import { createFileRoute } from "@tanstack/react-router"
import { SettingsFormActions } from "@/components/settings-form-actions"
import { Panel, PanelBody, PanelHeader } from "@/components/ui/panel"
import { Switch } from "@/components/ui/switch"
import { useConfigDraft } from "@/contexts/config-draft"

export const Route = createFileRoute("/general/")({
  component: GeneralSettings,
})

/**
 * General settings page component
 * Displays general preferences for the application
 */
function GeneralSettings() {
  const { draft, setDraft } = useConfigDraft()

  return (
    <Panel>
      <PanelHeader>
        <div className="flex flex-col gap-0.5">
          <h2 className="font-semibold text-[18px]">General</h2>
          <p className="text-[12px] text-foreground-subtle">
            General preferences for everyday use.
          </p>
        </div>
      </PanelHeader>
      <PanelBody>
        <div className="rounded-[10px] border border-border bg-background-elevated">
          <div className="flex h-[72px] items-center justify-between px-5">
            <div className="flex flex-col gap-1">
              <span className="font-medium text-[14px]">
                Enable Zero Gesture
              </span>
              <span className="text-[12px] text-foreground-subtle">
                Run gesture control on all of the other apps
              </span>
            </div>
            <Switch
              isSelected={draft.shared.enabled}
              onChange={(enabled) =>
                setDraft({
                  ...draft,
                  shared: { ...draft.shared, enabled },
                })
              }
            />
          </div>
        </div>
      </PanelBody>
      <SettingsFormActions />
    </Panel>
  )
}
