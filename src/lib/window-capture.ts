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

export function acceptedCurrentWindowCapture(
  active: boolean,
  current: WindowCaptureToken | null,
  requested: WindowCaptureToken,
  received: WindowCapturePoll,
): ForegroundWindowInfo | null {
  if (!isCurrentWindowCapture(active, current, requested)) {
    return null
  }
  return acceptedWindowCapture(requested, received)
}

export function isCurrentWindowCapture(
  active: boolean,
  current: WindowCaptureToken | null,
  requested: WindowCaptureToken,
): boolean {
  return (
    active &&
    current?.capture_id === requested.capture_id &&
    current.epoch === requested.epoch
  )
}
