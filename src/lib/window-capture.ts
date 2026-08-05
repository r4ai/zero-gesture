import type {
  ForegroundWindowInfo,
  WindowCapturePoll,
  WindowCaptureToken,
} from "@/lib/api"

export function acceptedWindowCapture(
  active: WindowCaptureToken,
  received: WindowCapturePoll,
): ForegroundWindowInfo | null {
  if (
    active.capture_id !== received.capture_id ||
    active.epoch !== received.epoch ||
    received.state !== "captured"
  ) {
    return null
  }
  return received.info
}
