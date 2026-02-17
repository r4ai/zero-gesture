import type { Meta, StoryObj } from "@storybook/react"
import { Check, Crosshair } from "lucide-react"
import { useState } from "react"
import { Button } from "./button"
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogHint,
  DialogIcon,
  PickWindowDialog,
  type PickWindowDialogProps,
} from "./dialog"

const meta: Meta<typeof Dialog> = {
  title: "UI/Dialog",
  component: Dialog,
  parameters: {
    layout: "centered",
  },
  tags: ["autodocs"],
}

export default meta

type Story = StoryObj<typeof Dialog>

/**
 * Basic dialog with header, body, and footer.
 * Demonstrates the compound component pattern.
 */
export const Default: Story = {
  render: () => {
    const [isOpen, setIsOpen] = useState(false)
    return (
      <>
        <Button onPress={() => setIsOpen(true)}>Open Dialog</Button>
        <Dialog isOpen={isOpen} onOpenChange={setIsOpen}>
          <Button onPress={() => setIsOpen(true)}>Open Dialog</Button>
          <DialogContent isDismissable>
            <DialogHeader>
              <DialogClose onPress={() => setIsOpen(false)} />
            </DialogHeader>
            <DialogBody>
              <p className="text-foreground">
                This is a basic dialog with header, body, and footer sections.
              </p>
            </DialogBody>
            <DialogFooter>
              <Button variant="outline" onPress={() => setIsOpen(false)}>
                Cancel
              </Button>
              <Button onPress={() => setIsOpen(false)}>Confirm</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </>
    )
  },
}

/**
 * Dialog with icon and description text.
 * Matches the "Pick-From-Screen: Step 1" design from settings.pen.
 */
export const WithIcon: Story = {
  render: () => {
    const [isOpen, setIsOpen] = useState(false)
    return (
      <>
        <Button onPress={() => setIsOpen(true)}>Open Icon Dialog</Button>
        <Dialog isOpen={isOpen} onOpenChange={setIsOpen}>
          <Button onPress={() => setIsOpen(true)}>Open Icon Dialog</Button>
          <DialogContent isDismissable>
            <DialogHeader>
              <DialogClose onPress={() => setIsOpen(false)} />
            </DialogHeader>
            <DialogBody>
              <DialogIcon>
                <Crosshair className="h-[34px] w-[34px] text-white" />
              </DialogIcon>
              <DialogDescription>
                Move your mouse over the app you want to add, then click on it.
              </DialogDescription>
              <DialogHint keyText="Esc" label="Press Esc to cancel" />
            </DialogBody>
          </DialogContent>
        </Dialog>
      </>
    )
  },
}

/**
 * Pre-configured PickWindowDialog component.
 * This is a convenience component for the "Pick From Screen" modal
 * that matches the design from settings.pen exactly.
 */
export const PickWindow: StoryObj<PickWindowDialogProps> = {
  render: () => {
    const [isOpen, setIsOpen] = useState(false)
    return (
      <>
        <Button onPress={() => setIsOpen(true)}>Pick From Screen</Button>
        <PickWindowDialog
          isOpen={isOpen}
          onOpenChange={setIsOpen}
          onClose={() => setIsOpen(false)}
        />
      </>
    )
  },
}

/**
 * Dialog without dismiss on outside click.
 * Requires explicit action to close.
 */
export const NonDismissable: Story = {
  render: () => {
    const [isOpen, setIsOpen] = useState(false)
    return (
      <>
        <Button onPress={() => setIsOpen(true)}>Open Non-Dismissable</Button>
        <Dialog isOpen={isOpen} onOpenChange={setIsOpen}>
          <Button onPress={() => setIsOpen(true)}>Open Non-Dismissable</Button>
          <DialogContent>
            <DialogHeader>
              <DialogClose onPress={() => setIsOpen(false)} />
            </DialogHeader>
            <DialogBody>
              <DialogDescription>
                This dialog cannot be dismissed by clicking outside. You must
                use the close button or confirm action.
              </DialogDescription>
            </DialogBody>
            <DialogFooter>
              <Button variant="outline" onPress={() => setIsOpen(false)}>
                Cancel
              </Button>
              <Button onPress={() => setIsOpen(false)}>
                <Check className="h-4 w-4" />
                Confirm
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </>
    )
  },
}

/**
 * Dialog with custom footer actions.
 */
export const CustomFooter: Story = {
  render: () => {
    const [isOpen, setIsOpen] = useState(false)
    return (
      <>
        <Button onPress={() => setIsOpen(true)}>Open Custom Footer</Button>
        <Dialog isOpen={isOpen} onOpenChange={setIsOpen}>
          <Button onPress={() => setIsOpen(true)}>Open Custom Footer</Button>
          <DialogContent isDismissable>
            <DialogHeader>
              <DialogClose onPress={() => setIsOpen(false)} />
            </DialogHeader>
            <DialogBody>
              <DialogDescription>
                This dialog has custom footer actions with multiple buttons.
              </DialogDescription>
            </DialogBody>
            <DialogFooter>
              <Button variant="ghost" onPress={() => setIsOpen(false)}>
                Skip
              </Button>
              <Button variant="outline" onPress={() => setIsOpen(false)}>
                Save as Draft
              </Button>
              <Button onPress={() => setIsOpen(false)}>Publish</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </>
    )
  },
}
