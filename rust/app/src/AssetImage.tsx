import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const cache = new Map<string, string>();

export function AssetImage({
  path,
  size = 40,
  radius = 6,
  title,
}: {
  path?: string;
  size?: number;
  radius?: number;
  title?: string;
}) {
  const [url, setUrl] = useState<string | undefined>(() =>
    path ? cache.get(path) : undefined,
  );

  useEffect(() => {
    if (!path) {
      setUrl(undefined);
      return;
    }
    const hit = cache.get(path);
    if (hit) {
      setUrl(hit);
      return;
    }
    let disposed = false;
    void invoke<string>("asset", { path })
      .then((value) => {
        if (cache.size > 600) {
          const oldest = cache.keys().next().value;
          if (oldest) cache.delete(oldest);
        }
        cache.set(path, value);
        if (!disposed) setUrl(value);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, [path]);

  const style = { width: size, height: size, borderRadius: radius, flexShrink: 0 };
  return url ? (
    <img
      src={url}
      alt={title ?? ""}
      title={title}
      style={{ ...style, objectFit: "cover" }}
    />
  ) : (
    <div style={{ ...style, background: "rgba(128,128,128,0.18)" }} />
  );
}