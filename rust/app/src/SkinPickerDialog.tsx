import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Input,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { ArrowLeftRegular } from "@fluentui/react-icons";
import { AssetImage } from "./AssetImage";

export interface SkinItem {
  id: number;
  name: string;
  splash?: string;
}

const useStyles = makeStyles({
  surface: { maxWidth: "860px", width: "92vw" },
  content: { display: "flex", flexDirection: "column", gap: "10px" },
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))",
    gap: "10px",
    maxHeight: "420px",
    overflowY: "auto",
    padding: "4px",
  },
  tile: {
    padding: 0,
    border: "2px solid transparent",
    borderRadius: "8px",
    background: "transparent",
    cursor: "pointer",
    display: "flex",
    flexDirection: "column",
    gap: "4px",
    textAlign: "left",
    color: "inherit",
    font: "inherit",
  },
  preview: {
    display: "flex",
    flexDirection: "column",
    gap: "10px",
  },
  previewHead: {
    display: "flex",
    alignItems: "center",
    gap: "8px",
  },
  muted: { color: tokens.colorNeutralForeground3 },
});

export function SkinPickerDialog({
  open,
  title,
  skins,
  onPick,
  onClose,
}: {
  open: boolean;
  title: string;
  skins: SkinItem[];
  onPick: (id: number) => void;
  onClose: () => void;
}) {
  const styles = useStyles();
  const [query, setQuery] = useState("");
  const [preview, setPreview] = useState<SkinItem | null>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setPreview(null);
    }
  }, [open]);

  const visible = useMemo(() => {
    const text = query.trim();
    if (!text) return skins;
    return skins.filter((skin) => skin.name.includes(text));
  }, [skins, query]);

  return (
    <Dialog
      open={open}
      onOpenChange={(_, data) => {
        if (!data.open) onClose();
      }}
    >
      <DialogSurface className={styles.surface}>
        <DialogBody>
          <DialogTitle>{title}</DialogTitle>
          <DialogContent className={styles.content}>
            {preview ? (
              <div className={styles.preview}>
                <div className={styles.previewHead}>
                  <Button
                    size="small"
                    appearance="subtle"
                    icon={<ArrowLeftRegular />}
                    onClick={() => setPreview(null)}
                  >
                    返回
                  </Button>
                  <Caption1>{preview.name}</Caption1>
                </div>
                <AssetImage
                  path={preview.splash}
                  width="100%"
                  height={340}
                  radius={10}
                  title={preview.name}
                />
              </div>
            ) : (
              <>
                <Input
                  value={query}
                  placeholder="搜索皮肤名称"
                  onChange={(_, data) => setQuery(data.value)}
                />
                <div className={styles.grid}>
                  {visible.map((skin) => (
                    <button
                      key={skin.id}
                      type="button"
                      className={styles.tile}
                      onClick={() => setPreview(skin)}
                    >
                      <AssetImage
                        path={skin.splash}
                        width="100%"
                        height={80}
                        radius={6}
                        title={skin.name}
                      />
                      <Caption1>{skin.name}</Caption1>
                    </button>
                  ))}
                </div>
                {visible.length === 0 && (
                  <Caption1 className={styles.muted}>没有匹配的皮肤。</Caption1>
                )}
              </>
            )}
          </DialogContent>
          <DialogActions>
            {preview && (
              <Button
                appearance="primary"
                onClick={() => {
                  onPick(preview.id);
                  onClose();
                }}
              >
                确定
              </Button>
            )}
            <Button appearance="secondary" onClick={onClose}>
              取消
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}