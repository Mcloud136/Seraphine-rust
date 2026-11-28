import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { friendlyError } from "./errors";
import {
  Badge,
  Button,
  Caption1,
  MessageBar,
  MessageBarBody,
  Tab,
  TabList,
  Text,
  Title3,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { AssetImage } from "./AssetImage";

interface RankEntry {
  tier?: string;
  division?: string;
  lp?: number;
  wins?: number;
  losses?: number;
}

interface RecentGame {
  gameId: number;
  queueId: number;
  gameCreation: number;
  gameDuration: number;
  championId: number;
  championName: string;
  kills: number;
  deaths: number;
  assists: number;
  win: boolean;
  remake: boolean;
  cs: number;
}

interface RecentSummary {
  games: RecentGame[];
  kills: number;
  deaths: number;
  assists: number;
  wins: number;
  losses: number;
  recentChampionId: number;
  recentChampionName: string;
  fateFlag: "ally" | "enemy" | null;
}

interface PlayerInfo {
  summonerId: number;
  puuid: string;
  name: string;
  tagLine: string;
  level: number;
  privacy: string;
  championId: number;
  championName: string;
  cellId?: number;
  position: string;
  rank: { solo?: RankEntry | null; flex?: RankEntry | null };
  recent: RecentSummary;
}

interface GameInfoPayload {
  side: string;
  queueId: number;
  isAram: boolean;
  summoners: PlayerInfo[];
}

interface AramBuff {
  championId: number;
  name: string;
  description: string;
  damageDealt?: number | null;
  damageTaken?: number | null;
  healing?: number | null;
  shielding?: number | null;
  abilityHaste?: number | null;
  tenacity?: number | null;
}

interface AramMeta {
  version: string;
  source: string;
}

interface ChampSelectMember {
  cellId?: number;
  championId?: number;
  summonerId?: number;
  assignedPosition?: string;
  nameVisibilityType?: string;
}

interface ChampSelectAction {
  actorCellId?: number;
  championId?: number;
  completed?: boolean;
  type?: string;
  isInProgress?: boolean;
}

interface ChampSelectSession {
  timer?: { phase?: string; adjustedTimeLeftInPhase?: number };
  localPlayerCellId?: number;
  myTeam?: ChampSelectMember[];
  theirTeam?: ChampSelectMember[];
  actions?: ChampSelectAction[][];
}

interface LivePlayer {
  summonerName: string;
  championName: string;
  championId: number;
  level: number;
  team: string;
  isBot: boolean;
  kills: number;
  deaths: number;
  assists: number;
  cs: number;
  items: number[];
  isActive: boolean;
}

interface LiveGame {
  gameTime: number;
  gameMode: string;
  mapName: string;
  gameVersion: string;
  activeSummoner: string;
  players: LivePlayer[];
}

const useStyles = makeStyles({
  page: { display: "flex", flexDirection: "column", gap: "14px" },
  top: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "12px",
  },
  list: { display: "flex", flexDirection: "column", gap: "8px" },
  card: {
    display: "grid",
    gridTemplateColumns: "56px 1fr auto",
    gap: "12px",
    alignItems: "center",
    padding: "10px 14px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  champion: {
    width: "56px",
    height: "56px",
    borderRadius: "8px",
    objectFit: "cover",
  },
  info: { display: "flex", flexDirection: "column", gap: "2px", minWidth: 0 },
  rankLine: { display: "flex", gap: "12px", flexWrap: "wrap" },
  recentIcons: { display: "flex", gap: "3px", flexWrap: "wrap" },
  nameLink: {
    padding: 0,
    border: "none",
    background: "transparent",
    color: "inherit",
    font: "inherit",
    cursor: "pointer",
    textAlign: "left",
  },
  recentGame: {
    padding: 0,
    border: "none",
    background: "transparent",
    cursor: "pointer",
    display: "inline-flex",
  },
  stats: { textAlign: "right", display: "flex", flexDirection: "column", gap: "4px" },
  muted: { color: tokens.colorNeutralForeground3 },
  bp: {
    padding: "12px 14px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "8px",
  },
  bpHeader: {
    display: "flex",
    alignItems: "center",
    gap: "12px",
    flexWrap: "wrap",
  },
  bpTeam: {
    display: "flex",
    flexDirection: "column",
    gap: "4px",
  },
  bpSlots: { display: "grid", gridTemplateColumns: "repeat(5, 1fr)", gap: "6px" },
  bpSlot: {
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    gap: "2px",
    padding: "6px 4px",
    borderRadius: "6px",
    background: tokens.colorNeutralBackground3,
    border: "1px solid transparent",
  },
  bpSlotLocal: { border: `1px solid ${tokens.colorBrandStroke1}` },
  liveRow: {
    display: "grid",
    gridTemplateColumns: "28px 1fr auto auto",
    gap: "6px",
    alignItems: "center",
    padding: "4px 6px",
    borderRadius: "6px",
    background: tokens.colorNeutralBackground3,
    border: "1px solid transparent",
  },
  liveActive: { border: `1px solid ${tokens.colorBrandStroke1}` },
  liveItems: { display: "flex", gap: "2px" },
  banIcons: { display: "flex", gap: "3px", flexWrap: "wrap", alignItems: "center" },
  empty: {
    padding: "48px 12px",
    textAlign: "center",
    display: "grid",
    gap: "10px",
    justifyItems: "center",
  },
});

const POSITION_LABELS: Record<string, string> = {
  TOP: "上单",
  JUNGLE: "打野",
  MIDDLE: "中单",
  BOTTOM: "下路",
  UTILITY: "辅助",
};

type Styles = ReturnType<typeof useStyles>;

function liveTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

function LiveGameCard({
  live,
  items: itemCatalog,
  styles,
}: {
  live: LiveGame;
  items: Record<string, { iconPath?: string }>;
  styles: Styles;
}) {
  const teams = [
    { label: "蓝色方", team: "ORDER" },
    { label: "红色方", team: "CHAOS" },
  ];
  return (
    <div className={styles.bp}>
      <div className={styles.bpHeader}>
        <Text weight="semibold">对局进行中 · {liveTime(live.gameTime)}</Text>
        <Caption1 className={styles.muted}>
          {live.mapName} · {live.gameMode}
        </Caption1>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
        {teams.map((entry) => (
          <div className={styles.bpTeam} key={entry.team}>
            <Caption1 className={styles.muted}>{entry.label}</Caption1>
            {live.players
              .filter((player) => player.team === entry.team)
              .map((player) => (
                <div
                  className={`${styles.liveRow} ${
                    player.isActive ? styles.liveActive : ""
                  }`}
                  key={player.summonerName}
                >
                  <AssetImage
                    path={championIcon(player.championId)}
                    size={28}
                    title={player.championName}
                  />
                  <div style={{ minWidth: 0 }}>
                    <Text weight={player.isActive ? "semibold" : "regular"}>
                      {player.summonerName}
                      {player.isBot ? " (AI)" : ""}
                    </Text>
                    <br />
                    <Caption1 className={styles.muted}>
                      Lv.{player.level} · {player.championName}
                    </Caption1>
                  </div>
                  <div style={{ textAlign: "right" }}>
                    <Text weight="semibold">
                      {player.kills}/{player.deaths}/{player.assists}
                    </Text>
                    <br />
                    <Caption1 className={styles.muted}>{player.cs} CS</Caption1>
                  </div>
                  <div className={styles.liveItems}>
                    {player.items.map((itemId, index) => (
                      <AssetImage
                        key={`${player.summonerName}-${index}`}
                        path={itemCatalog[String(itemId)]?.iconPath}
                        size={20}
                        radius={3}
                      />
                    ))}
                  </div>
                </div>
              ))}
          </div>
        ))}
      </div>
    </div>
  );
}

function phaseLabel(phase?: string): string {
  switch (phase) {
    case "PLANNING":
      return "计划";
    case "BAN_PICK":
      return "禁用/选择";
    case "FINALIZATION":
      return "最终确认";
    case "GAME_STARTING":
      return "即将开始";
    default:
      return phase ?? "准备中";
  }
}

function ChampSelectCard({
  session,
  champions,
  styles,
}: {
  session: ChampSelectSession;
  champions: Record<string, string>;
  styles: Styles;
}) {
  const myTeam = [...(session.myTeam ?? [])].sort(
    (left, right) => (left.cellId ?? 0) - (right.cellId ?? 0),
  );
  const theirTeam = [...(session.theirTeam ?? [])].sort(
    (left, right) => (left.cellId ?? 0) - (right.cellId ?? 0),
  );
  const bans = (session.actions ?? [])
    .flat()
    .filter(
      (action) =>
        action.type === "ban" &&
        action.completed &&
        (action.championId ?? 0) > 0,
    );
  const seconds = Math.max(
    0,
    Math.ceil((session.timer?.adjustedTimeLeftInPhase ?? 0) / 1000),
  );

  return (
    <div className={styles.bp}>
      <div className={styles.bpHeader}>
        <Text weight="semibold">
          英雄选择 · {phaseLabel(session.timer?.phase)}
        </Text>
        <Caption1 className={styles.muted}>剩余 {seconds}s</Caption1>
        <Caption1 className={styles.muted}>
          只读展示；禁用/选用由“游戏功能”页的自动化执行
        </Caption1>
      </div>
      <TeamSlots
        label="己方"
        team={myTeam}
        localCell={session.localPlayerCellId}
        champions={champions}
        styles={styles}
      />
      <TeamSlots
        label="敌方"
        team={theirTeam}
        champions={champions}
        styles={styles}
      />
      {bans.length > 0 && (
        <div className={styles.banIcons}>
          <Caption1 className={styles.muted}>禁用</Caption1>
          {bans.map((ban, index) => (
            <AssetImage
              key={`ban-${index}`}
              path={championIcon(ban.championId ?? 0)}
              size={22}
              title={champions[String(ban.championId)]}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function TeamSlots({
  label,
  team,
  localCell,
  champions,
  styles,
}: {
  label: string;
  team: ChampSelectMember[];
  localCell?: number;
  champions: Record<string, string>;
  styles: Styles;
}) {
  return (
    <div className={styles.bpTeam}>
      <Caption1 className={styles.muted}>{label}</Caption1>
      <div className={styles.bpSlots}>
        {team.map((member) => {
          const championId = member.championId ?? 0;
          const name = champions[String(championId)] ?? "";
          return (
            <div
              key={member.cellId}
              className={`${styles.bpSlot} ${
                member.cellId === localCell ? styles.bpSlotLocal : ""
              }`}
            >
              <AssetImage
                path={championIcon(championId)}
                size={32}
                title={name}
              />
              <Caption1>
                {championId > 0
                  ? name || `#${championId}`
                  : member.nameVisibilityType === "HIDDEN"
                    ? "隐藏"
                    : "未选择"}
              </Caption1>
            </div>
          );
        })}
        {team.length === 0 && (
          <Caption1 className={styles.muted}>暂无</Caption1>
        )}
      </div>
    </div>
  );
}

function rankText(entry: RankEntry | null | undefined): string {
  if (!entry || !entry.tier) return "未定级";
  return `${entry.tier} ${entry.division ?? ""}`.trim();
}

function championIcon(championId: number): string | undefined {
  return championId > 0
    ? `/lol-game-data/assets/v1/champion-icons/${championId}.png`
    : undefined;
}

export default function GameInfoPage({
  connected,
  reserveGameInfo = false,
  showTier = false,
  onOpenSearch,
}: {
  connected: boolean;
  reserveGameInfo?: boolean;
  showTier?: boolean;
  onOpenSearch: (request: { query: string; gameId?: number }) => void;
}) {
  const styles = useStyles();
  const [side, setSide] = useState("ally");
  const [ally, setAlly] = useState<GameInfoPayload | null>(null);
  const [enemy, setEnemy] = useState<GameInfoPayload | null>(null);
  const [error, setError] = useState("");
  const [buffs, setBuffs] = useState<Record<number, AramBuff | null>>({});
  const [aramMeta, setAramMeta] = useState<AramMeta | null>(null);
  const [champSelect, setChampSelect] = useState<ChampSelectSession | null>(null);
  const [live, setLive] = useState<LiveGame | null>(null);
  const [champions, setChampions] = useState<Record<string, string>>({});
  const [items, setItems] = useState<Record<string, { iconPath?: string }>>({});
  const [refreshTick, setRefreshTick] = useState(0);

  useEffect(() => {
    if (!connected) {
      setLive(null);
      setChampSelect(null);
      setBuffs({});
      setAramMeta(null);
      setError("");
      if (!reserveGameInfo) {
        setAlly(null);
        setEnemy(null);
      }
      return;
    }
    let disposed = false;
    async function load() {
      try {
        const results = await Promise.allSettled([
          invoke<GameInfoPayload>("gameinfo_fetch", { side: "ally" }),
          invoke<GameInfoPayload>("gameinfo_fetch", { side: "enemy" }),
          invoke<ChampSelectSession | null>("champselect_state"),
          invoke<LiveGame | null>("live_game"),
        ]);
        if (disposed) return;
        const allyResult = results[0];
        const enemyResult = results[1];
        const champSelectResult = results[2];
        const liveResult = results[3];
        if (allyResult.status === "fulfilled") {
          setAlly(allyResult.value);
        }
        if (enemyResult.status === "fulfilled") {
          setEnemy(enemyResult.value);
        }
        if (champSelectResult.status === "fulfilled") {
          setChampSelect(champSelectResult.value);
        }
        if (liveResult.status === "fulfilled") {
          setLive(liveResult.value);
        }
        if (allyResult.status === "rejected") {
          setError(
            friendlyError(
              allyResult.reason,
              "己方对局信息读取失败，请确认客户端处于选人或对局中。",
            ),
          );
        } else if (enemyResult.status === "rejected") {
          setError(
            friendlyError(
              enemyResult.reason,
              "敌方对局信息读取失败，请确认客户端处于选人或对局中。",
            ),
          );
        } else {
          setError("");
        }
      } catch (cause) {
        if (!disposed)
          setError(
            friendlyError(cause, "读取对局信息失败，请稍后重试。"),
          );
      }
    }
    void load();
    const timer = window.setInterval(() => void load(), 2000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [connected, reserveGameInfo, refreshTick]);

  useEffect(() => {
    if (!connected) return;
    void invoke<{
      champions?: Record<string, string>;
      items?: Record<string, { iconPath?: string }>;
    }>("catalog_get")
      .then((catalog) => {
        setChampions(catalog.champions ?? {});
        setItems(catalog.items ?? {});
      })
      .catch(() => undefined);
  }, [connected]);

  const payload = side === "ally" ? ally : enemy;
  const summoners = payload?.summoners ?? [];
  const isAram = ally?.isAram ?? false;

  useEffect(() => {
    if (!isAram) return;
    void invoke<AramMeta>("aram_meta")
      .then(setAramMeta)
      .catch(() => undefined);
  }, [isAram]);

  useEffect(() => {
    if (!isAram) return;
    const ids = new Set<number>();
    for (const player of [...(ally?.summoners ?? []), ...(enemy?.summoners ?? [])]) {
      if (player.championId > 0) ids.add(player.championId);
    }
    for (const id of ids) {
      if (buffs[id] !== undefined) continue;
      void invoke<AramBuff | null>("aram_champion", { championId: id })
        .then((buff) => setBuffs((previous) => ({ ...previous, [id]: buff })))
        .catch(() => undefined);
    }
  }, [isAram, ally, enemy, buffs]);

  return (
    <div className={styles.page}>
      <div className={styles.top}>
        <Title3>对局信息</Title3>
        <TabList
          selectedValue={side}
          onTabSelect={(_, data) => setSide(String(data.value))}
        >
          <Tab value="ally">己方</Tab>
          <Tab value="enemy">敌方</Tab>
        </TabList>
        {isAram && (
          <Badge appearance="tint" color="brand">
            极地大乱斗
          </Badge>
        )}
        {aramMeta && (
          <Caption1 className={styles.muted}>
            ARAM 数据 v{aramMeta.version}
          </Caption1>
        )}
      </div>

      {error && (
        <MessageBar intent="error">
          <MessageBarBody>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <span>{error}</span>
              <Button
                size="small"
                onClick={() => setRefreshTick((tick) => tick + 1)}
              >
                重试
              </Button>
            </div>
          </MessageBarBody>
        </MessageBar>
      )}

      {connected && live && (
        <LiveGameCard live={live} items={items} styles={styles} />
      )}

      {connected && champSelect && (
        <ChampSelectCard
          session={champSelect}
          champions={champions}
          styles={styles}
        />
      )}

      {summoners.length === 0 && (
        <div className={styles.empty}>
          <Title3>
            {side === "enemy" ? "敌方信息暂不可用" : "等待英雄选择或对局"}
          </Title3>
          <Caption1 className={styles.muted}>
            {side === "enemy"
              ? "进入对局后，敌方召唤师信息会在这里显示。"
              : "进入英雄选择后自动加载；也可在对局中查看。"}
          </Caption1>
        </div>
      )}

      <div className={styles.list}>
        {summoners.map((player) => {
          const kda = player.recent;
          const games = kda.games.slice(0, 10);
          const buff = isAram ? buffs[player.championId] : null;
          return (
            <div className={styles.card} key={player.summonerId}>
              <AssetImage
                path={championIcon(player.championId)}
                size={56}
                title={player.championName}
              />
              <div className={styles.info}>
                <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                  <button
                    type="button"
                    className={styles.nameLink}
                    title="在搜索中查看该玩家"
                    onClick={() =>
                      onOpenSearch({
                        query:
                          player.puuid ||
                          `${player.name}${player.tagLine ? `#${player.tagLine}` : ""}`,
                      })
                    }
                  >
                    <Text weight="semibold">
                      {player.name}
                      {player.tagLine ? `#${player.tagLine}` : ""}
                    </Text>
                  </button>
                  <Caption1 className={styles.muted}>
                    Lv.{player.level}
                  </Caption1>
                  {player.position && (
                    <Caption1 className={styles.muted}>
                      {POSITION_LABELS[player.position] ?? player.position}
                    </Caption1>
                  )}
                  {kda.fateFlag === "ally" && (
                    <Badge appearance="tint" color="success">
                      上局队友
                    </Badge>
                  )}
                  {kda.fateFlag === "enemy" && (
                    <Badge appearance="tint" color="danger">
                      上局对手
                    </Badge>
                  )}
                </div>
                {showTier && (
                  <div className={styles.rankLine}>
                    <Caption1>
                      单双排 {rankText(player.rank?.solo)}
                      {player.rank?.solo
                        ? ` · ${player.rank.solo.lp ?? 0} LP`
                        : ""}
                    </Caption1>
                    <Caption1>
                      灵活 {rankText(player.rank?.flex)}
                      {player.rank?.flex
                        ? ` · ${player.rank.flex.lp ?? 0} LP`
                        : ""}
                    </Caption1>
                  </div>
                )}
                <div className={styles.recentIcons}>
                  {games.map((game) => (
                    <button
                      type="button"
                      key={game.gameId}
                      className={styles.recentGame}
                      title={`${game.championName} ${game.kills}/${game.deaths}/${game.assists} ${
                        game.win ? "胜" : "负"
                      } · 在搜索中查看`}
                      onClick={() =>
                        onOpenSearch({
                          query:
                            player.puuid ||
                            `${player.name}${player.tagLine ? `#${player.tagLine}` : ""}`,
                          gameId: game.gameId,
                        })
                      }
                      style={{
                        borderBottom: `2px solid ${
                          game.win
                            ? tokens.colorPaletteGreenBorder1
                            : tokens.colorPaletteRedBorder1
                        }`,
                      }}
                    >
                      <AssetImage
                        path={championIcon(game.championId)}
                        size={24}
                        radius={4}
                      />
                    </button>
                  ))}
                </div>
                {buff?.description && (
                  <Caption1 className={styles.muted} title={buff.description}>
                    {buff.description}
                  </Caption1>
                )}
              </div>
              <div className={styles.stats}>
                <Text weight="semibold">
                  {kda.kills} / {kda.deaths} / {kda.assists}
                </Text>
                <Caption1>
                  {kda.wins}胜 {kda.losses}负
                </Caption1>
                {kda.recentChampionName && (
                  <Caption1 className={styles.muted}>
                    最近使用 {kda.recentChampionName}
                  </Caption1>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}