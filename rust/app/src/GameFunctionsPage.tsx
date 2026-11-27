import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Button,
  Caption1,
  Dropdown,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Option,
  Switch,
  Title3,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";
import { AssetImage } from "./AssetImage";
import { IconPickerDialog, type PickerItem } from "./IconPickerDialog";
import { friendlyError } from "./errors";
import { CHAMPION_KEYWORDS } from "./championAliases";
import type { AppConfig, FunctionConfig } from "./config";

const useStyles = makeStyles({
  page: { display: "flex", flexDirection: "column", gap: "12px" },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "12px",
  },
  grid: {
    display: "block",
    columnGap: "12px",
    columnCount: 1,
    "@media (min-width: 720px)": { columnCount: 2 },
    "@media (min-width: 1180px)": { columnCount: 3 },
  },
  card: {
    padding: "14px 16px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "10px",
    minWidth: 0,
    width: "100%",
    maxWidth: "368px",
    breakInside: "avoid",
    marginBottom: "12px",
  },
  muted: { color: tokens.colorNeutralForeground3 },
});

const BAN_KEYS = {
  general: "autoBanChampion",
  TOP: "autoBanChampionTop",
  JUNGLE: "autoBanChampionJug",
  MID: "autoBanChampionMid",
  ADC: "autoBanChampionBot",
  SUPPORT: "autoBanChampionSup",
} as const;

const SELECT_KEYS = {
  general: "autoSelectChampion",
  TOP: "autoSelectChampionTop",
  JUNGLE: "autoSelectChampionJug",
  MID: "autoSelectChampionMid",
  ADC: "autoSelectChampionBot",
  SUPPORT: "autoSelectChampionSup",
} as const;

const SPELL_KEYS = {
  general: "autoSetSummonerSpell",
  TOP: "autoSetSummonerSpellTop",
  JUNGLE: "autoSetSummonerSpellJug",
  MID: "autoSetSummonerSpellMid",
  ADC: "autoSetSummonerSpellBot",
  SUPPORT: "autoSetSummonerSpellSup",
} as const;

const POSITION_LABELS: Record<string, string> = {
  general: "通用",
  TOP: "上单",
  JUNGLE: "打野",
  MID: "中单",
  ADC: "下路",
  SUPPORT: "辅助",
};

export default function GameFunctionsPage({
  config,
  onSaved,
}: {
  config: AppConfig;
  onSaved: (config: AppConfig) => void;
}) {
  const styles = useStyles();
  const [draft, setDraft] = useState<FunctionConfig>(config.functions);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState("");
  const [listPosition, setListPosition] = useState("general");
  const [champions, setChampions] = useState<Record<string, string>>({});
  const [spells, setSpells] = useState<
    Record<string, { name?: string; iconPath?: string }>
  >({});
  const [picker, setPicker] = useState<{
    title: string;
    items: PickerItem[];
    selected: number[];
    multiple: boolean;
    onConfirm: (ids: number[]) => void;
  } | null>(null);

  useEffect(() => {
    setDraft(config.functions);
  }, [config]);

  useEffect(() => {
    void invoke<{
      champions?: Record<string, string>;
      spells?: Record<string, { name?: string; iconPath?: string }>;
    }>("catalog_get")
      .then((catalog) => {
        setChampions(catalog.champions ?? {});
        setSpells(catalog.spells ?? {});
      })
      .catch(() => undefined);
  }, []);

  const championItems = useMemo<PickerItem[]>(
    () =>
      Object.entries(champions).map(([id, name]) => ({
        id: Number(id),
        name,
        icon: `/lol-game-data/assets/v1/champion-icons/${id}.png`,
        keywords: CHAMPION_KEYWORDS[Number(id)] ?? [],
      })),
    [champions],
  );

  const spellItems = useMemo<PickerItem[]>(
    () =>
      Object.entries(spells)
        .map(([id, spell]) => ({
          id: Number(id),
          name: spell.name || `技能 ${id}`,
          icon: spell.iconPath,
        }))
        .filter((spell) => spell.id !== 54 && spell.name !== "占位"),
    [spells],
  );

  function update<K extends keyof FunctionConfig>(
    key: K,
    value: FunctionConfig[K],
  ) {
    setDraft((current) => ({ ...current, [key]: value }));
  }

  async function save() {
    setSaving(true);
    setMessage("");
    try {
      const saved = await invoke<AppConfig>("config_patch", {
        patch: { functions: draft },
      });
      onSaved(saved);
      setMessage("已保存游戏功能设置。");
    } catch (cause) {
      setMessage(friendlyError(cause, "保存失败，请稍后重试。"));
    } finally {
      setSaving(false);
    }
  }

  const banKey = BAN_KEYS[listPosition as keyof typeof BAN_KEYS];
  const selectKey = SELECT_KEYS[listPosition as keyof typeof SELECT_KEYS];
  const spellKey = SPELL_KEYS[listPosition as keyof typeof SPELL_KEYS];
  const banList = (draft[banKey] as number[] | undefined) ?? [];
  const selectList = (draft[selectKey] as number[] | undefined) ?? [];
  const spellList = (draft[spellKey] as number[] | undefined) ?? [54, 54];

  function renderChampionList(listKey: string, values: number[]) {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
          {values.map((id) => (
            <Badge key={id} appearance="tint" color="informative">
              {champions[String(id)] ?? `#${id}`}
              <Button
                size="small"
                appearance="transparent"
                icon={<DismissRegular />}
                aria-label="移除"
                onClick={() =>
                  update(
                    listKey as keyof FunctionConfig,
                    values.filter((value) => value !== id) as never,
                  )
                }
              />
            </Badge>
          ))}
          {values.length === 0 && (
            <Caption1 className={styles.muted}>尚未选择英雄</Caption1>
          )}
        </div>
        <Button
          appearance="secondary"
          onClick={() =>
            setPicker({
              title: "选择英雄",
              items: championItems,
              selected: values,
              multiple: true,
              onConfirm: (ids) =>
                update(listKey as keyof FunctionConfig, ids as never),
            })
          }
        >
          选择英雄…
        </Button>
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <Title3>游戏功能</Title3>
        <Button appearance="primary" disabled={saving} onClick={() => void save()}>
          {saving ? "保存中…" : "保存设置"}
        </Button>
      </div>
      {message && (
        <MessageBar intent={message.startsWith("已") ? "success" : "error"}>
          <MessageBarBody>{message}</MessageBarBody>
        </MessageBar>
      )}

      <div className={styles.grid}>
        <section className={styles.card}>
          <Title3>对局与自动化</Title3>
          <Field label="职业战绩场次">
            <Input
              type="number"
              value={String(draft.careerGamesNumber)}
              onChange={(_, data) =>
                update("careerGamesNumber", Number(data.value) || 0)
              }
            />
          </Field>
          <Field label="自动接受对局">
            <Switch
              checked={draft.enableAutoAcceptMatching}
              onChange={(_, data) =>
                update("enableAutoAcceptMatching", data.checked)
              }
            />
          </Field>
          <Field label="自动接受延迟（秒）">
            <Input
              type="number"
              value={String(draft.autoAcceptMatchingDelay)}
              onChange={(_, data) =>
                update("autoAcceptMatchingDelay", Number(data.value) || 0)
              }
            />
          </Field>
          <Field label="自动重连">
            <Switch
              checked={draft.enableAutoReconnect}
              onChange={(_, data) => update("enableAutoReconnect", data.checked)}
            />
          </Field>
          <Field label="自动接受换位（楼层交换）">
            <Switch
              checked={draft.autoAcceptCeilSwap}
              onChange={(_, data) => update("autoAcceptCeilSwap", data.checked)}
            />
          </Field>
          <Field label="自动接受英雄交换">
            <Switch
              checked={draft.autoAcceptChampTrade}
              onChange={(_, data) =>
                update("autoAcceptChampTrade", data.checked)
              }
            />
          </Field>
          <Field label="选人超时自动锁定">
            <Switch
              checked={draft.enableAutoSelectTimeoutCompleted}
              onChange={(_, data) =>
                update("enableAutoSelectTimeoutCompleted", data.checked)
              }
            />
          </Field>
          <Field label="禁用时避开队友预选">
            <Switch
              checked={draft.prententBan}
              onChange={(_, data) => update("prententBan", data.checked)}
            />
          </Field>
        </section>

        <section className={styles.card}>
          <Title3>自动禁用 / 自动选用名单</Title3>
          <Field label="分路">
            <Dropdown
              value={POSITION_LABELS[listPosition]}
              selectedOptions={[listPosition]}
              onOptionSelect={(_, data) =>
                setListPosition(data.optionValue ?? "general")
              }
            >
              {Object.entries(POSITION_LABELS).map(([value, label]) => (
                <Option key={value} value={value}>
                  {label}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Field label="自动禁用英雄">
            <Switch
              checked={draft.enableAutoBanChampion}
              onChange={(_, data) =>
                update("enableAutoBanChampion", data.checked)
              }
            />
          </Field>
          <Field label="自动禁用延迟（秒）">
            <Input
              type="number"
              value={String(draft.autoBanDelay)}
              onChange={(_, data) =>
                update("autoBanDelay", Number(data.value) || 0)
              }
            />
          </Field>
          <Caption1 className={styles.muted}>自动禁用名单</Caption1>
          {renderChampionList(banKey, banList)}
          <Field label="自动选用英雄">
            <Switch
              checked={draft.enableAutoSelectChampion}
              onChange={(_, data) =>
                update("enableAutoSelectChampion", data.checked)
              }
            />
          </Field>
          <Caption1 className={styles.muted}>自动选用名单</Caption1>
          {renderChampionList(selectKey, selectList)}
          <Field label="自动设置召唤师技能">
            <Switch
              checked={draft.enableAutoSetSpells}
              onChange={(_, data) => update("enableAutoSetSpells", data.checked)}
            />
          </Field>
          <Caption1 className={styles.muted}>自动召唤师技能（D / F）</Caption1>
          <div style={{ display: "flex", gap: 8 }}>
            {[0, 1].map((index) => {
              const spellId = spellList[index] ?? 54;
              const spell = spells[String(spellId)];
              const empty =
                spellId === 54 || !spell?.name || spell.name === "占位";
              return (
                <Button
                  key={`spell-${index}`}
                  appearance="secondary"
                  icon={
                    <AssetImage
                      path={empty ? undefined : spell?.iconPath}
                      size={20}
                      radius={4}
                      title={empty ? "未选择" : spell?.name}
                    />
                  }
                  onClick={() =>
                    setPicker({
                      title: `选择召唤师技能（${index === 0 ? "D" : "F"}）`,
                      items: spellItems,
                      selected: [spellId],
                      multiple: false,
                      onConfirm: (ids) => {
                        if (!ids.length) return;
                        const next = [
                          spellList[0] ?? 54,
                          spellList[1] ?? 54,
                        ];
                        next[index] = ids[0];
                        update(spellKey as keyof FunctionConfig, next as never);
                      },
                    })
                  }
                >
                  {empty ? "未选择" : spell?.name}
                </Button>
              );
            })}
          </div>
          <Caption1 className={styles.muted}>
            按分路保存；实际写操作仍受策略开关约束。
          </Caption1>
        </section>
      </div>

      {picker && (
        <IconPickerDialog
          open
          title={picker.title}
          items={picker.items}
          selected={picker.selected}
          multiple={picker.multiple}
          onConfirm={picker.onConfirm}
          onClose={() => setPicker(null)}
        />
      )}
    </div>
  );
}