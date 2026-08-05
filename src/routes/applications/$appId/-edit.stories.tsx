import type { Meta, StoryObj } from "@storybook/react"
import {
  createMemoryHistory,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router"
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"
import { routeTree } from "@/routeTree.gen"
import { DEFAULTS } from "@/types/config"

const invokeMock = fn((command: string) => {
  switch (command) {
    case "get_config":
      return {
        revision: 1,
        generation: 1,
        config: {
          ...DEFAULTS,
          applications: [
            {
              platform: "windows",
              application: {
                id: "test-app",
                label: "Test App",
                matchers: [
                  {
                    target: "process_name",
                    method: "exact",
                    value: "old.exe",
                  },
                ],
              },
            },
          ],
        },
      }
    case "start_window_capture":
      return { capture_id: 1, epoch: 1 }
    case "poll_window_capture":
      return {
        state: "captured",
        info: {
          process_name: "explorer.exe",
          window_class: "CabinetWClass",
          title: "Files",
        },
      }
    case "stop_window_capture":
      return undefined
    default:
      throw new Error(`unexpected command: ${command}`)
  }
})

function AppEditPageStory() {
  const router = createRouter({
    routeTree,
    history: createMemoryHistory({
      initialEntries: ["/applications/test-app/edit"],
    }),
  })
  return <RouterProvider router={router} />
}

const meta = {
  title: "Routes/AppEditPage",
  component: AppEditPageStory,
  beforeEach: () => {
    invokeMock.mockClear()
    Object.assign(globalThis, { isTauri: true })
    mockIPC((command) => invokeMock(command))
    return () => {
      clearMocks()
      Reflect.deleteProperty(globalThis, "isTauri")
    }
  },
} satisfies Meta<typeof AppEditPageStory>

export default meta
type Story = StoryObj<typeof meta>

export const SharesOneCaptureController: Story = {
  play: async ({ canvasElement }) => {
    const page = within(canvasElement.ownerDocument.body)
    await userEvent.click(
      await page.findByRole("button", {
        name: "Capture From Screen for condition 1",
      }),
    )

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(
          ([command]) => command === "start_window_capture",
        ),
      ).toHaveLength(1)
    })
    await expect(await page.findByText("explorer")).toBeVisible()
  },
}
