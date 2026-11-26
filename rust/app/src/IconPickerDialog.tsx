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
import { DismissRegular } from "@fluentui/react-icons";
import { AssetImage } from "./AssetImage";

export interface PickerItem {
  id: number;
  name: string;
  icon?: string;
}

const useStyles = makeStyles({
  surface: { maxWidth: "780px", width: "92vw" },
  content: { display: "flex", flexDirection: "column", gap: "10px" },
  chosen: { display: "flex", gap: "6px", flexWrap: "wrap", minHeight: "28px" },
  chip: {
    display: "flex",
    alignItems: "center",
    gap: "4px",
    padding: "2px 4px 2px 6px",
    borderRadius: "6px",
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    background: tokens.colorNeutralBackground2,
  },
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fill, minmax(48px, 1fr))",
    gap: "8px",
    maxHeight: "340px",
    overflowY: "auto",
    padding: "4px",
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    borderRadius: "8px",
  },
  pick: {
    padding: 0,
    border: "2px solid transparent",
    borderRadius: "8px",
    background: "transparent",
    cursor: "pointer",
    display: "grid",
    placeItems: "center",
  },
  picked: {
    padding: 0,
    borderRadius: "8px",
    cursor: "pointer",
    display: "grid",
    placeItems: "center",
    border: `2px solid ${tokens.colorBrandStroke1}`,
    background: tokens.colorBrandBackground2,
  },
  muted: { color: tokens.colorNeutralForeground3 },
});

export function IconPickerDialog({
  open,
  title,
  items,
  selected,
  multiple,
  onConfirm,
  onClose,
}: {
  open: boolean;
  title: string;
  items: PickerItem[];
  selected: number[];
  multiple: boolean;
  onConfirm: (ids: number[]) => void;
  onClose: () => void;
}) {
  const styles = useStyles();
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState<number[]>(selected);

  useEffect(() => {
    if (open) {
      setDraft(selected);
      setQuery("");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const filtered = useMemo(() => {
    const text = query.trim();
    return items
      .filter(
        (item) =>
          !text ||
          item.name.includes(text) ||
          String(item.id).includes(text),
      )
      .sort((left, right) => left.name.localeCompare(right.name, "zh-CN"));
  }, [items, query]);

  const chosen = items.filter((item) => draft.includes(item.id));

  function toggle(id: number) {
    if (multiple) {
      setDraft((current) =>
        current.includes(id)
          ? current.filter((value) => value !== id)
          : [...current, id],
      );
    } else {
      setDraft([id]);
    }
  }

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
            {multiple && (
              <div className={styles.chosen}>
                {chosen.map((item) => (
                  <span key={item.id} className={styles.chip}>
                    <AssetImage
                      path={item.icon}
                      size={20}
                      radius={4}
                      title={item.name}
                    />
                    <Caption1>{item.name}</Caption1>
                    <Button
                      size="small"
                      appearance="transparent"
                      icon={<DismissRegular />}
                      aria-label={`移除 ${item.name}`}
                      onClick={() => toggle(item.id)}
                    />
                  </span>
                ))}
                {chosen.length === 0 && (
                  <Caption1 className={styles.muted}>尚未选择</Caption1>
                )}
              </div>
            )}
            <Input
              value={query}
              placeholder="搜索名称或 ID"
              onChange={(_, data) => setQuery(data.value)}
            />
            <div className={styles.grid}>
              {filtered.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className={
                    draft.includes(item.id) ? styles.picked : styles.pick
                  }
                  title={`${item.name} #${item.id}`}
                  aria-label={item.name}
                  onClick={() => toggle(item.id)}
                >
                  <AssetImage
                    path={item.icon}
                    size={40}
                    radius={6}
                    title={item.name}
                  />
                </button>
              ))}
            </div>
            {filtered.length === 0 && (
              <Caption1 className={styles.muted}>没有匹配项。</Caption1>
            )}
          </DialogContent>
          <DialogActions>
            <Button
              appearance="primary"
              onClick={() => {
                onConfirm(draft);
                onClose();
              }}
            >
              确定
            </Button>
            <Button appearance="secondary" onClick={onClose}>
              取消
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}