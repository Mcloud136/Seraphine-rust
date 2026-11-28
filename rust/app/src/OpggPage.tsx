import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { friendlyError } from "./errors";
import {
  Badge,
  Button,
  Caption1,
  Dropdown,
  MessageBar,
  MessageBarBody,
  Option,
  Spinner,
  Text,
  Title3,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { ArrowClockwiseRegular } from "@fluentui/react-icons";
import { AssetImage } from "./AssetImage";

interface OpggEntry {
  championId?: number;
  name?: string;
  position?: string;
  winRate?: number | null;
  pickRate?: number | null;
  banRate?: number | null;
  kda?: number | null;
  tier?: number | string | null;
  rank?: number | string | null;
}

interface TierPayload {
  data: Record<string, OpggEntry[]> | OpggEntry[];
  version?: string | null;
}

interface OpggItemEntry {
  icons?: string[];
  play?: number;
  win?: number;
  pickRate?: number;
}

interface OpggPerk {
  primaryIcon?: string;
  secondaryIcon?: string;
  icons?: string[];
  pickRate?: number;
}

interface OpggBuild {
  summary?: OpggEntry;
  championSkills?: {
    order?: number[];
    play?: number;
    win?: number;
    pickRate?: number;
  } | null;
  items?: {
    boots?: OpggItemEntry[];
    startItems?: OpggItemEntry[];
    coreItems?: OpggItemEntry[];
    lastItems?: string[];
    prismItems?: OpggItemEntry[];
  };
  perks?: OpggPerk[];
  classicRunes?: { runes?: { icon?: string; name?: string; count?: number }[] }[];
  counters?: {
    strongAgainst?: { championId?: number; name?: string; winRate?: number }[];
    weakAgainst?: { championId?: number; name?: string; winRate?: number }[];
  };
  augments?: unknown[] | null;
  synergies?: {
    championId?: number;
    name?: string;
    win?: number;
    play?: number;
    pickRate?: number;
  }[];
}

interface OpggAugment {
  id?: number;
  name?: string;
  icon?: string;
  pickRate?: number;
  win?: number;
  play?: number;
}

interface BuildPayload {
  data: OpggBuild;
  version?: string | null;
  mode?: string;
  position?: string;
}

const REGIONS = [
  { value: "kr", label: "韩服" },
  { value: "global", label: "全球" },
];

const MODES = [
  { value: "ranked", label: "排位" },
  { value: "aram", label: "极地大乱斗" },
  { value: "arena", label: "斗魂竞技场" },
  { value: "urf", label: "无限火力" },
  { value: "nexus_blitz", label: "极限闪击" },
  { value: "classic", label: "匹配" },
];

const TIERS = [
  { value: "all", label: "全部段位" },
  { value: "ibsg", label: "铁铜银金" },
  { value: "gold_plus", label: "黄金+" },
  { value: "platinum_plus", label: "铂金+" },
  { value: "emerald_plus", label: "翡翠+" },
  { value: "diamond_plus", label: "钻石+" },
  { value: "master", label: "大师" },
  { value: "master_plus", label: "大师+" },
  { value: "grandmaster", label: "宗师+" },
  { value: "challenger", label: "王者" },
];

const POSITIONS = [
  { value: "TOP", label: "上单" },
  { value: "JUNGLE", label: "打野" },
  { value: "MID", label: "中单" },
  { value: "ADC", label: "下路" },
  { value: "SUPPORT", label: "辅助" },
];

const useStyles = makeStyles({
  page: { display: "flex", flexDirection: "column", gap: "12px" },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "12px",
  },
  controls: { display: "flex", gap: "8px", flexWrap: "wrap", alignItems: "center" },
  table: {
    display: "flex",
    flexDirection: "column",
    gap: "2px",
  },
  row: {
    display: "grid",
    gridTemplateColumns: "36px 44px 1fr 70px 70px 70px 60px 56px",
    gap: "8px",
    alignItems: "center",
    padding: "6px 10px",
    borderRadius: "6px",
    background: tokens.colorNeutralBackground2,
    cursor: "pointer",
  },
  head: {
    background: "transparent",
    color: tokens.colorNeutralForeground3,
    cursor: "default",
  },
  build: {
    padding: "14px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "10px",
  },
  buildHeader: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "8px",
  },
  section: { display: "flex", flexDirection: "column", gap: "4px" },
  icons: { display: "flex", gap: "3px", flexWrap: "wrap", alignItems: "center" },
  group: {
    display: "flex",
    flexDirection: "column",
    gap: "2px",
    padding: "6px 8px",
    borderRadius: "6px",
    background: tokens.colorNeutralBackground3,
  },
  name: { minWidth: 0 },
  numeric: { textAlign: "right", fontVariantNumeric: "tabular-nums" },
  muted: { color: tokens.colorNeutralForeground3 },
  empty: {
    padding: "40px 12px",
    textAlign: "center",
    display: "grid",
    gap: "8px",
    justifyItems: "center",
  },
});

function percentText(value: number | null | undefined): string {
  if (value === null || value === undefined) return "-";
  const normalized = value > 0 && value <= 1 ? value * 100 : value;
  return `${normalized.toFixed(2)}%`;
}

function numberText(value: number | null | undefined, digits = 2): string {
  if (value === null || value === undefined) return "-";
  return value.toFixed(digits);
}

function tierText(value: number | string | null | undefined): string {
  if (value === null || value === undefined || value === "") return "";
  return String(value).toUpperCase();
}

function OpggIcon({ spec, size = 26 }: { spec?: string; size?: number }) {
  if (!spec) {
    return (
      <div
        style={{
          width: size,
          height: size,
          borderRadius: 4,
          background: "rgba(128,128,128,0.18)",
          flexShrink: 0,
        }}
      />
    );
  }
  if (spec.startsWith("lcu:")) {
    return <AssetImage path={spec.slice(4)} size={size} radius={4} />;
  }
  if (spec.startsWith("http")) {
    return (
      <img
        src={spec}
        alt=""
        style={{ width: size, height: size, borderRadius: 4, flexShrink: 0 }}
      />
    );
  }
  return (
    <div
      style={{
        width: size,
        height: size,
        borderRadius: 4,
        background: "rgba(128,128,128,0.18)",
        flexShrink: 0,
      }}
    />
  );
}

const SKILL_LABELS: Record<number, string> = {
  1: "Q",
  2: "W",
  3: "E",
  4: "R",
};

export default function OpggPage({ connected }: { connected: boolean }) {
  const styles = useStyles();
  const [region, setRegion] = useState("kr");
  const [mode, setMode] = useState("ranked");
  const [tier, setTier] = useState("emerald_plus");
  const [position, setPosition] = useState("ADC");
  const [payload, setPayload] = useState<TierPayload | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [build, setBuild] = useState<BuildPayload | null>(null);
  const [buildName, setBuildName] = useState("");
  const [buildLoading, setBuildLoading] = useState(false);
  const [buildError, setBuildError] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const data = await invoke<TierPayload>("opgg_tier_list", {
        region,
        mode,
        tier,
      });
      setPayload(data);
    } catch (cause) {
      setPayload(null);
      setError(
        friendlyError(cause, "加载 OP.GG 数据失败，请稍后重试。"),
      );
    } finally {
      setLoading(false);
    }
  }, [region, mode, tier]);

  useEffect(() => {
    if (!connected) return;
    void load();
  }, [connected, load]);

  const entries = useMemo(() => {
    if (!payload) return [];
    if (Array.isArray(payload.data)) return payload.data;
    return payload.data[position] ?? [];
  }, [payload, position]);

  async function openBuild(entry: OpggEntry) {
    if (!entry.championId) return;
    setBuildLoading(true);
    setBuildError("");
    setBuildName(entry.name ?? "");
    try {
      const result = await invoke<BuildPayload>("opgg_champion_build", {
        region,
        mode,
        tier,
        championId: entry.championId,
        position: mode === "ranked" ? position : "none",
      });
      setBuild(result);
    } catch (cause) {
      setBuild(null);
      setBuildError(
        friendlyError(cause, "获取出装数据失败，请稍后重试。"),
      );
    } finally {
      setBuildLoading(false);
    }
  }

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <Title3>OP.GG 榜单</Title3>
        <Button
          appearance="subtle"
          icon={<ArrowClockwiseRegular />}
          disabled={loading || !connected}
          onClick={() => void load()}
        >
          刷新
        </Button>
      </div>

      <div className={styles.controls}>
        <Dropdown
          aria-label="区服"
          value={REGIONS.find((item) => item.value === region)?.label ?? region}
          selectedOptions={[region]}
          onOptionSelect={(_, data) => setRegion(data.optionValue ?? "kr")}
        >
          {REGIONS.map((item) => (
            <Option key={item.value} value={item.value}>
              {item.label}
            </Option>
          ))}
        </Dropdown>
        <Dropdown
          aria-label="模式"
          value={MODES.find((item) => item.value === mode)?.label ?? mode}
          selectedOptions={[mode]}
          onOptionSelect={(_, data) => setMode(data.optionValue ?? "ranked")}
        >
          {MODES.map((item) => (
            <Option key={item.value} value={item.value}>
              {item.label}
            </Option>
          ))}
        </Dropdown>
        <Dropdown
          aria-label="段位"
          value={TIERS.find((item) => item.value === tier)?.label ?? tier}
          selectedOptions={[tier]}
          onOptionSelect={(_, data) => setTier(data.optionValue ?? "all")}
        >
          {TIERS.map((item) => (
            <Option key={item.value} value={item.value}>
              {item.label}
            </Option>
          ))}
        </Dropdown>
        {mode === "ranked" && (
          <Dropdown
            aria-label="位置"
            value={
              POSITIONS.find((item) => item.value === position)?.label ?? position
            }
            selectedOptions={[position]}
            onOptionSelect={(_, data) => setPosition(data.optionValue ?? "ADC")}
          >
            {POSITIONS.map((item) => (
              <Option key={item.value} value={item.value}>
                {item.label}
              </Option>
            ))}
          </Dropdown>
        )}
        {payload?.version && (
          <Caption1 className={styles.muted}>数据版本 {payload.version}</Caption1>
        )}
      </div>

      {error && (
        <MessageBar intent="error">
          <MessageBarBody>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <span>{error}</span>
              <Button size="small" onClick={() => void load()}>
                重试
              </Button>
            </div>
          </MessageBarBody>
        </MessageBar>
      )}

      {buildLoading && <Spinner label={`读取 ${buildName} 出装…`} />}
      {buildError && (
        <MessageBar intent="error">
          <MessageBarBody>{buildError}</MessageBarBody>
        </MessageBar>
      )}
      {build && !buildLoading && (
        <div className={styles.build}>
          <div className={styles.buildHeader}>
            <div>
              <Text weight="semibold" size={400}>
                {build.data.summary?.name ?? buildName}
              </Text>
              {build.data.summary && (
                <Caption1 className={styles.muted}>
                  {" "}
                  · 胜率 {percentText(build.data.summary.winRate)} · 选取率{" "}
                  {percentText(build.data.summary.pickRate)} · Ban率{" "}
                  {percentText(build.data.summary.banRate)}
                </Caption1>
              )}
            </div>
            <Button appearance="subtle" onClick={() => setBuild(null)}>
              收起
            </Button>
          </div>

          {build.data.items && (
            <div className={styles.section}>
              <Caption1 className={styles.muted}>装备路线</Caption1>
              {(
                [
                  ["鞋子", build.data.items.boots],
                  ["起始装", build.data.items.startItems],
                  ["核心装", build.data.items.coreItems],
                  ["棱彩", build.data.items.prismItems],
                ] as [string, OpggItemEntry[] | undefined][]
              )
                .filter(([, items]) => items && items.length > 0)
                .map(([label, items]) => (
                  <div className={styles.group} key={label}>
                    <Caption1 className={styles.muted}>{label}</Caption1>
                    {items!.slice(0, 3).map((item, index) => (
                      <div className={styles.icons} key={`${label}-${index}`}>
                        {(item.icons ?? []).map((icon, iconIndex) => (
                          <OpggIcon key={`${label}-${index}-${iconIndex}`} spec={icon} />
                        ))}
                        <Caption1 className={styles.muted}>
                          选取 {percentText(item.pickRate)}
                        </Caption1>
                      </div>
                    ))}
                  </div>
                ))}
              {build.data.items.lastItems &&
                build.data.items.lastItems.length > 0 && (
                  <div className={styles.icons}>
                    <Caption1 className={styles.muted}>后期装</Caption1>
                    {build.data.items.lastItems.map((icon, index) => (
                      <OpggIcon key={`last-${index}`} spec={icon} size={24} />
                    ))}
                  </div>
                )}
            </div>
          )}

          {build.data.perks && build.data.perks.length > 0 && (
            <div className={styles.section}>
              <Caption1 className={styles.muted}>符文</Caption1>
              <div className={styles.icons}>
                <OpggIcon spec={build.data.perks[0].primaryIcon} size={30} />
                <OpggIcon spec={build.data.perks[0].secondaryIcon} size={24} />
                {(build.data.perks[0].icons ?? []).map((icon, index) => (
                  <OpggIcon key={`perk-${index}`} spec={icon} size={22} />
                ))}
                <Caption1 className={styles.muted}>
                  选取 {percentText(build.data.perks[0].pickRate)}
                </Caption1>
              </div>
            </div>
          )}

          {build.data.classicRunes?.[0]?.runes &&
            build.data.classicRunes[0].runes.length > 0 && (
              <div className={styles.section}>
                <Caption1 className={styles.muted}>经典符文</Caption1>
                <div className={styles.icons}>
                  {build.data.classicRunes[0].runes.map((rune, index) => (
                    <OpggIcon key={`classic-${index}`} spec={rune.icon} size={24} />
                  ))}
                </div>
              </div>
            )}

          {build.data.championSkills?.order &&
            build.data.championSkills.order.length > 0 && (
              <div className={styles.section}>
                <Caption1 className={styles.muted}>技能顺序</Caption1>
                <div className={styles.icons}>
                  {build.data.championSkills.order.map((skill, index) => (
                    <Badge key={`skill-${index}`} appearance="tint" color="informative">
                      {SKILL_LABELS[skill] ?? skill}
                    </Badge>
                  ))}
                </div>
              </div>
            )}

          {build.data.counters && (
            <div className={styles.section}>
              <Caption1 className={styles.muted}>克制关系</Caption1>
              <div className={styles.icons}>
                {(build.data.counters.strongAgainst ?? []).slice(0, 6).map((counter) => (
                  <div key={`strong-${counter.championId}`} title={counter.name}>
                    <AssetImage
                      path={
                        counter.championId
                          ? `/lol-game-data/assets/v1/champion-icons/${counter.championId}.png`
                          : undefined
                      }
                      size={26}
                    />
                  </div>
                ))}
              </div>
              <div className={styles.icons}>
                {(build.data.counters.weakAgainst ?? []).slice(0, 6).map((counter) => (
                  <div key={`weak-${counter.championId}`} title={counter.name}>
                    <AssetImage
                      path={
                        counter.championId
                          ? `/lol-game-data/assets/v1/champion-icons/${counter.championId}.png`
                          : undefined
                      }
                      size={26}
                    />
                  </div>
                ))}
              </div>
            </div>
          )}

          {Array.isArray(build.data.augments) && build.data.augments.length > 0 && (
            <div className={styles.section}>
              <Caption1 className={styles.muted}>强化符文</Caption1>
              {(
                build.data.augments as OpggAugment[][]
              ).map((group, groupIndex) => (
                <div className={styles.icons} key={`augment-group-${groupIndex}`}>
                  {group.slice(0, 4).map((augment) => (
                    <div key={`augment-${groupIndex}-${augment.id}`}>
                      <OpggIcon spec={augment.icon} size={26} />
                      <Caption1 className={styles.muted}>
                        {augment.name ?? ""}
                      </Caption1>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          )}

          {build.data.synergies && build.data.synergies.length > 0 && (
            <div className={styles.section}>
              <Caption1 className={styles.muted}>协同英雄</Caption1>
              <div className={styles.icons}>
                {build.data.synergies.slice(0, 8).map((synergy) => (
                  <div
                    key={`synergy-${synergy.championId}`}
                    title={synergy.name}
                    style={{ textAlign: "center" }}
                  >
                    <AssetImage
                      path={
                        synergy.championId
                          ? `/lol-game-data/assets/v1/champion-icons/${synergy.championId}.png`
                          : undefined
                      }
                      size={26}
                    />
                    <Caption1 className={styles.muted}>
                      {synergy.pickRate
                        ? percentText(synergy.pickRate)
                        : ""}
                    </Caption1>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {!connected && (
        <div className={styles.empty}>
          <Title3>等待英雄联盟客户端</Title3>
        </div>
      )}
      {loading && <Spinner label="读取 OP.GG 数据…" />}

      {!loading && connected && entries.length > 0 && (
        <div className={styles.table}>
          <div className={`${styles.row} ${styles.head}`}>
            <span>#</span>
            <span />
            <span>英雄</span>
            <span className={styles.numeric}>胜率</span>
            <span className={styles.numeric}>选取率</span>
            <span className={styles.numeric}>Ban率</span>
            <span className={styles.numeric}>KDA</span>
            <span />
          </div>
          {entries.map((entry, index) => (
            <div
              className={styles.row}
              key={`${entry.championId}-${index}`}
              onClick={() => void openBuild(entry)}
            >
              <Caption1>{String(entry.rank ?? index + 1)}</Caption1>
              <AssetImage
                path={
                  entry.championId
                    ? `/lol-game-data/assets/v1/champion-icons/${entry.championId}.png`
                    : undefined
                }
                size={40}
              />
              <div className={styles.name}>
                <Text weight="semibold">{entry.name ?? "未知英雄"}</Text>
                {entry.tier && (
                  <Badge appearance="tint" color="brand" style={{ marginLeft: 8 }}>
                    {tierText(entry.tier)}
                  </Badge>
                )}
              </div>
              <span className={styles.numeric}>{percentText(entry.winRate)}</span>
              <span className={styles.numeric}>
                {percentText(entry.pickRate)}
              </span>
              <span className={styles.numeric}>{percentText(entry.banRate)}</span>
              <span className={styles.numeric}>{numberText(entry.kda)}</span>
              <Button
                size="small"
                appearance="subtle"
                onClick={(event) => {
                  event.stopPropagation();
                  void openBuild(entry);
                }}
              >
                出装
              </Button>
            </div>
          ))}
        </div>
      )}

      {!loading && connected && entries.length === 0 && !error && (
        <div className={styles.empty}>
          <Caption1 className={styles.muted}>当前筛选没有数据。</Caption1>
        </div>
      )}
    </div>
  );
}