import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

export function useCatalogRefresh(refresh: () => void | Promise<void>) {
  useEffect(() => {
    const unlisten = listen("catalog://refreshed", () => void refresh());
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [refresh]);
}