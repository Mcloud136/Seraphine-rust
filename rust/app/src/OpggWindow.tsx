import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FluentProvider, Spinner } from "@fluentui/react-components";
import OpggPage from "./OpggPage";
import { applyZoom, buildTheme } from "./theme";
import type { AppConfig } from "./config";

export default function OpggWindow() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [connected, setConnected] = useState(false);
  const theme = useMemo(() => buildTheme(config), [config]);

  useEffect(() => {
    let disposed = false;
    void invoke<AppConfig>("config_get")
      .then((value) => {
        if (!disposed) setConfig(value);
      })
      .catch(() => undefined);
    const refresh = () =>
      void invoke<{ connected: boolean }>("session_status")
        .then((value) => {
          if (!disposed) setConnected(Boolean(value.connected));
        })
        .catch(() => undefined);
    refresh();
    const timer = window.setInterval(refresh, 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    void applyZoom(config?.personalization.dpiScale);
  }, [config?.personalization.dpiScale]);

  return (
    <FluentProvider theme={theme} style={{ height: "100%" }}>
      <div
        style={{
          height: "100%",
          overflowY: "auto",
          padding: 16,
          boxSizing: "border-box",
        }}
      >
        {config ? (
          <OpggPage
            connected={connected}
            defaults={{
              region: config.functions.opggRegion,
              tier: config.functions.opggTier,
              position: config.functions.opggPosition,
            }}
          />
        ) : (
          <Spinner label="读取 OP.GG 设置…" />
        )}
      </div>
    </FluentProvider>
  );
}