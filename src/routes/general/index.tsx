import { createFileRoute } from "@tanstack/react-router"
import { Button } from "../../components/ui/button"
import {
  Panel,
  PanelBody,
  PanelFooter,
  PanelHeader,
} from "../../components/ui/panel"
import { Select, SelectItem } from "../../components/ui/select"
import { Switch } from "../../components/ui/switch"
import { TextField } from "../../components/ui/textfield"

export const Route = createFileRoute("/general/")({
  component: GeneralSettings,
})

function GeneralSettings() {
  return (
    <Panel>
      <PanelHeader>
        <h2 className="font-semibold text-lg">General Settings</h2>
      </PanelHeader>
      <PanelBody>
        <div className="max-w-2xl space-y-6">
          <div className="space-y-4">
            <h3 className="font-medium text-lg">Appearance</h3>
            <div className="grid gap-4">
              <Select label="Theme" placeholder="Select theme">
                <SelectItem id="system">System</SelectItem>
                <SelectItem id="light">Light</SelectItem>
                <SelectItem id="dark">Dark</SelectItem>
              </Select>
              <Select label="Language" placeholder="Select language">
                <SelectItem id="en">English</SelectItem>
                <SelectItem id="ja">Japanese</SelectItem>
              </Select>
            </div>
          </div>

          <div className="space-y-4">
            <h3 className="font-medium text-lg">Behavior</h3>
            <div className="flex items-center justify-between rounded-lg border p-4">
              <div className="space-y-0.5">
                <span className="font-medium text-base">Enable Gestures</span>
                <p className="text-muted-foreground text-sm">
                  Turn mouse gestures on or off globally.
                </p>
              </div>
              <Switch defaultSelected aria-label="Enable Gestures" />
            </div>

            <TextField
              label="Display Name"
              placeholder="Enter your name"
              description="This name will be used for personalization."
            />
          </div>
        </div>
      </PanelBody>
      <PanelFooter>
        <div className="flex w-full justify-end gap-2">
          <Button variant="outline">Reset</Button>
          <Button>Save Changes</Button>
        </div>
      </PanelFooter>
    </Panel>
  )
}
