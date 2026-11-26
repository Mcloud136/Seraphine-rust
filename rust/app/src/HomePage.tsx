import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Button,
  Caption1,
  MessageBar,
  MessageBarBody,
  Spinner,
  Subtitle1,
  Title3,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

interface SessionStatus {
  connected: boolean;
  pid: number;
  port: number;
  server: string;
  epoch: number;
  websocketReceived: number;
  websocketReconnects: number;
}

const useStyles = makeStyles({
  page: {
    display: "grid",
    placeItems: "center",
    minHeight: "100%",
  },
  card: {
    width: "100%",
    maxWidth: "368px",
    padding: "22px 24px",
    borderRadius: "12px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "14px",
  },
  head: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
  },
  rows: {
    display: "grid",
    gridTemplateColumns: "auto 1fr",
    gap: "6px 18px",
    fontVariantNumeric: "tabular-nums",
  },
  label: { color: tokens.colorNeutralForeground3 },
  actions: { display: "flex", gap: "8px", justifyContent: "flex-end" },
  muted: { color: tokens.colorNeutralForeground3 },
});

export default function HomePage() {
  const styles = useStyles();
  const [status, setStatus] = useState<SessionStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<SessionStatus>("session_status"));
    } catch {
      // keep the last known status; the next poll retries
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  async function connect() {
    setBusy(true);
    setMessage("");
    try {
      const clients = await invoke<{ pid: number }[]>("lcu_clients");
      const pid = clients[0]?.pid;
      if (!pid) {
        setMessage("未发现英雄联盟客户端进程。");
        return;
      }
      await invoke("session_connect", { pid });
      await refresh();
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function disconnect() {
    setBusy(true);
    setMessage("");
    try {
      await invoke("session_disconnect");
      await refresh();
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={styles.page}>
      <section className={styles.card}>
        <div className={styles.head}>
          <Title3>客户端连接</Title3>
          {status ? (
            <Badge
              appearance="tint"
              color={status.connected ? "success" : "danger"}
              style={{ flexShrink: 0 }}
            >
              {status.connected ? "已连接" : "未连接"}
            </Badge>
          ) : (
            <Spinner size="tiny" />
          )}
        </div>
        <Caption1 className={styles.muted}>
          启动客户端后会自动发现并连接。
        </Caption1>
        {status && (
          <div className={styles.rows}>
            <Caption1 className={styles.label}>区服</Caption1>
            <Subtitle1>{status.server || "未知"}</Subtitle1>
            <Caption1 className={styles.label}>进程</Caption1>
            <Caption1>
              {status.connected ? `PID ${status.pid}` : "—"}
            </Caption1>
            <Caption1 className={styles.label}>端口</Caption1>
            <Caption1>{status.connected ? status.port : "—"}</Caption1>
            <Caption1 className={styles.label}>事件</Caption1>
            <Caption1>
              {status.connected
                ? `${status.websocketReceived} 条 · 重连 ${status.websocketReconnects} 次`
                : "—"}
            </Caption1>
          </div>
        )}
        {message && (
          <MessageBar intent="error">
            <MessageBarBody>{message}</MessageBarBody>
          </MessageBar>
        )}
        <div className={styles.actions}>
          {status?.connected ? (
            <Button disabled={busy} onClick={() => void disconnect()}>
              断开连接
            </Button>
          ) : (
            <Button
              appearance="primary"
              disabled={busy}
              onClick={() => void connect()}
            >
              {busy ? "连接中…" : "立即连接"}
            </Button>
          )}
        </div>
      </section>
    </div>
  );
}