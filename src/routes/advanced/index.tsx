import { useMutation } from "@tanstack/react-query"
import { createFileRoute } from "@tanstack/react-router"
import { open, save } from "@tauri-apps/plugin-dialog"
import { Download, FolderOpen, Upload } from "lucide-react"
import { Button } from "@/components/ui/button"
import { exportConfig, importConfig, openConfigDir } from "@/lib/api"

function ImportConfigItem() {
  const { mutate, isPending } = useMutation({
    mutationFn: async () => {
      const filePath = await open({
        filters: [{ name: "JSON", extensions: ["json"] }],
        multiple: false,
      })
      if (filePath) await importConfig(filePath)
    },
  })

  return (
    <div className="flex items-center justify-between rounded-lg border px-5 py-4">
      <div className="space-y-2">
        <p className="font-semibold leading-none">Import Config</p>
        <p className="text-muted-foreground text-sm">
          Load a JSON config file and apply it as the current configuration.
        </p>
      </div>
      <Button
        variant="outline"
        className="ml-6 shrink-0"
        onClick={() => mutate()}
        disabled={isPending}
      >
        <Upload className="h-4 w-4" />
        Import
      </Button>
    </div>
  )
}

function ExportConfigItem() {
  const { mutate, isPending } = useMutation({
    mutationFn: async () => {
      const filePath = await save({
        filters: [{ name: "JSON", extensions: ["json"] }],
        defaultPath: "zero-gesture.config.json",
      })
      if (filePath) await exportConfig(filePath)
    },
  })

  return (
    <div className="flex items-center justify-between rounded-lg border px-5 py-4">
      <div className="space-y-2">
        <p className="font-semibold leading-none">Export Config</p>
        <p className="text-muted-foreground text-sm">
          Save the current configuration to a JSON file.
        </p>
      </div>
      <Button
        variant="outline"
        className="ml-6 shrink-0"
        onClick={() => mutate()}
        disabled={isPending}
      >
        <Download className="h-4 w-4" />
        Export
      </Button>
    </div>
  )
}

function OpenFolderItem() {
  const { mutate, isPending } = useMutation({
    mutationFn: openConfigDir,
  })

  return (
    <div className="flex items-center justify-between rounded-lg border px-5 py-4">
      <div className="space-y-2">
        <p className="font-semibold leading-none">Config File Location</p>
        <p className="text-muted-foreground text-sm">
          Open the folder where the config file is stored.
        </p>
      </div>
      <Button
        variant="outline"
        className="ml-6 shrink-0"
        onClick={() => mutate()}
        disabled={isPending}
      >
        <FolderOpen className="h-4 w-4" />
        Open Folder
      </Button>
    </div>
  )
}

function AdvancedPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-bold text-2xl tracking-tight">Advanced</h2>
        <p className="text-muted-foreground">
          Advanced configuration options for power users.
        </p>
      </div>

      <div className="space-y-4">
        <ImportConfigItem />
        <ExportConfigItem />
        <OpenFolderItem />
      </div>
    </div>
  )
}

export const Route = createFileRoute("/advanced/")({
  component: AdvancedPage,
})
