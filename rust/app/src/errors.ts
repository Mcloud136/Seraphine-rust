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
  if (/NotConnected|not connected|客户端未连接/i.test(detail)) {
    return "请先连接英雄联盟客户端。";
  }
  if (/SummonerNotFound|未找到召唤师/i.test(detail)) {
    return "未找到该玩家，请检查名称后重试。";
  }
  return message;
}