import { createFileRoute } from "@tanstack/react-router"
import { open, save } from "@tauri-apps/plugin-dialog"
import { Download, FolderOpen, Upload } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Panel, PanelBody, PanelHeader } from "@/components/ui/panel"
import { exportConfig, importConfig, openConfigDir } from "@/lib/api"

export const Route = createFileRoute("/advanced/")({
  component: AdvancedSettings,
})

/**
 * Advanced settings page component
 * Displays advanced configuration options for power users
 */
function AdvancedSettings() {
  const handleImportConfig = async () => {
    try {
      const filePath = await open({
        title: "Import Config",
        filters: [
          {
            name: "JSON",
            extensions: ["json"],
          },
        ],
      })

      if (filePath) {
        await importConfig(filePath)
        toast.success("Config imported successfully")
      }
    } catch (error) {
      console.error("Failed to import config:", error)
      toast.error("Failed to import config")
    }
  }

  const handleExportConfig = async () => {
    try {
      const filePath = await save({
        title: "Export Config",
        filters: [
          {
            name: "JSON",
            extensions: ["json"],
          },
        ],
        defaultPath: "zero-gesture.config.json",
      })

      if (filePath) {
        await exportConfig(filePath)
        toast.success("Config exported successfully")
      }
    } catch (error) {
      console.error("Failed to export config:", error)
      toast.error("Failed to export config")
    }
  }

  const handleOpenConfigFolder = async () => {
    try {
      await openConfigDir()
    } catch (error) {
      console.error("Failed to open config folder:", error)
      toast.error("Failed to open config folder")
    }
  }

  return (
    <Panel>
      <PanelHeader>
        <div className="flex flex-col gap-0.5">
          <h2 className="font-semibold text-[18px]">Advanced</h2>
          <p className="text-[12px] text-foreground-subtle">
            Advanced configuration options for power users.
          </p>
        </div>
      </PanelHeader>
      <PanelBody>
        <div className="rounded-[10px] border border-border bg-background-elevated">
          {/* Import Config Row */}
          <div className="flex h-[72px] items-center justify-between gap-4 border-border border-b px-5">
            <div className="flex flex-1 flex-col gap-1">
              <span className="font-medium text-[14px]">Import Config</span>
              <span className="text-[12px] text-foreground-subtle">
                Load a JSON config file and apply it as the current
                configuration.
              </span>
            </div>
            <Button
              variant="outline"
              className="h-[34px] gap-2 rounded-[7px] border-border bg-background-card px-[14px] text-foreground"
              onPress={handleImportConfig}
            >
              <Upload className="h-3.5 w-3.5" />
              <span className="font-medium text-[13px]">Import</span>
            </Button>
          </div>

          {/* Export Config Row */}
          <div className="flex h-[72px] items-center justify-between gap-4 border-border border-b px-5">
            <div className="flex flex-1 flex-col gap-1">
              <span className="font-medium text-[14px]">Export Config</span>
              <span className="text-[12px] text-foreground-subtle">
                Save the current configuration to a JSON file.
              </span>
            </div>
            <Button
              variant="outline"
              className="h-[34px] gap-2 rounded-[7px] border-border bg-background-card px-[14px] text-foreground"
              onPress={handleExportConfig}
            >
              <Download className="h-3.5 w-3.5" />
              <span className="font-medium text-[13px]">Export</span>
            </Button>
          </div>

          {/* Open Config Folder Row */}
          <div className="flex h-[72px] items-center justify-between gap-4 px-5">
            <div className="flex flex-1 flex-col gap-1">
              <span className="font-medium text-[14px]">Open Config</span>
              <span className="text-[12px] text-foreground-subtle">
                Open the folder where the config file is stored.
              </span>
            </div>
            <Button
              variant="outline"
              className="h-[34px] gap-2 rounded-[7px] border-border bg-background-card px-[14px] text-foreground"
              onPress={handleOpenConfigFolder}
            >
              <FolderOpen className="h-3.5 w-3.5" />
              <span className="font-medium text-[13px]">Open Folder</span>
            </Button>
          </div>
        </div>
      </PanelBody>
    </Panel>
  )
}
