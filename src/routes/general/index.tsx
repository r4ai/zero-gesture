import { createFileRoute } from "@tanstack/react-router"
import { useState } from "react"
import { Button } from "@/components/ui/button"
import {
  Panel,
  PanelBody,
  PanelFooter,
  PanelHeader,
} from "@/components/ui/panel"
import { Switch } from "@/components/ui/switch"

export const Route = createFileRoute("/general/")({
  component: GeneralSettings,
})

/**
 * General settings page component
 * Displays general preferences for the application
 */
function GeneralSettings() {
  const [enableZeroGesture, setEnableZeroGesture] = useState(true)

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
              isSelected={enableZeroGesture}
              onChange={setEnableZeroGesture}
            />
          </div>
        </div>
      </PanelBody>
      <PanelFooter>
        <Button variant="outline">Cancel</Button>
        <Button>
          <span className="font-semibold text-[13px]">Save Changes</span>
        </Button>
      </PanelFooter>
    </Panel>
  )
}
