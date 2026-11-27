import { invoke } from "@tauri-apps/api/core";

export const DEFAULT_ERROR = "操作失败，请稍后重试。";

/// Shows a friendly message to the user while the technical detail goes to the
/// application log (via the ui_log command) for diagnosis.
export function friendlyError(
  cause: unknown,
  message: string = DEFAULT_ERROR,
): string {
  const detail =
    cause instanceof Error
      ? `${cause.name}: ${cause.message}`
      : typeof cause === "string"
        ? cause
        : JSON.stringify(cause);
  void invoke("ui_log", { message: detail }).catch(() => undefined);
  return message;
}