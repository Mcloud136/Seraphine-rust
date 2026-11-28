import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Button,
  Caption1,
  Dropdown,
  Input,
  MessageBar,
  MessageBarBody,
  Option,
  Spinner,
  Subtitle1,
  Text,
  Title3,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { AssetImage } from "./AssetImage";
import { formatDuration, formatTime, rankText, winRate } from "./format";
import { friendlyError } from "./errors";

interface Summoner {
  gameName?: string;
  tagLine?: string;
  displayName?: string;
  summonerLevel?: number;
  profileIconId?: number;
  puuid?: string;
}

interface RankedEntry {
  tier?: string;
  division?: string;
  leaguePoints?: number;
  wins?: number;
  losses?: number;
}

interface MatchSummary {
  gameId: number;
  queueId: number;
  queueName: string;
  gameMode: string;
  gameCreation: number;
  gameDuration: number;
  championId: number;
  championName: string;
  win: boolean;
  kills: number;
  deaths: number;
  assists: number;
}

interface PlayerPayload {
  summoner: Summoner;
  ranked: { queueMap?: Record<string, RankedEntry> } | null;
  matches: MatchSummary[];
}

interface CatalogData {
  champions?: Record<string, string>;
  items?: Record<string, { iconPath?: string }>;
  spells?: Record<string, { iconPath?: string }>;
  runes?: Record<string, { icon?: string; name?: string }>;
  queues?: Record<string, { name?: string }>;
}

interface ParticipantStats {
  [key: string]: number | boolean | undefined;
  win?: boolean;
  kills?: number;
  deaths?: number;
  assists?: number;
  totalMinionsKilled?: number;
  neutralMinionsKilled?: number;
  totalDamageDealtToChampions?: number;
  perk0?: number;
}

interface GameParticipant {
  participantId: number;
  teamId: number;
  championId: number;
  spell1Id: number;
  spell2Id: number;
  stats: ParticipantStats;
}

interface GameIdentity {
  participantId: number;
  player?: {
    gameName?: string;
    tagLine?: string;
    summonerName?: string;
    puuid?: string;
  };
}

interface GameTeam {
  teamId: number;
  win: boolean;
  bans?: { championId: number }[];
}

interface TftGameSummary {
  gameId: number;
  placement: number;
  level: number;
  setNumber: number;
  duration: number;
  gameCreation: number;
  units?: {
    championId: number;
    championName: string;
    tier: number;
  }[];
  augments?: { id: number; name?: string }[];
}

interface GameDetail {
  gameId: number;
  gameCreation: number;
  gameDuration: number;
  queueId: number;
  gameMode: string;
  participants?: GameParticipant[];
  participantIdentities?: GameIdentity[];
  teams?: GameTeam[];
}

const useStyles = makeStyles({
  page: {
    display: "flex",
    flexDirection: "column",
    gap: "16px",
  },
  searchBar: {
    display: "flex",
    gap: "8px",
  },
  header: {
    display: "flex",
    gap: "14px",
    alignItems: "center",
  },
  rankRow: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
    gap: "10px",
  },
  card: {
    padding: "12px 14px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "grid",
    gap: "2px",
  },
  grid: {
    display: "grid",
    gridTemplateColumns: "minmax(360px, 1fr) minmax(380px, 1.1fr)",
    gap: "16px",
    alignItems: "start",
  },
  column: {
    display: "flex",
    flexDirection: "column",
    gap: "8px",
    minWidth: 0,
  },
  matchRow: {
    display: "grid",
    gridTemplateColumns: "40px 1fr auto",
    gap: "10px",
    alignItems: "center",
    padding: "8px 10px",
    borderRadius: "8px",
    background: tokens.colorNeutralBackground2,
    borderLeftWidth: "3px",
    borderLeftStyle: "solid",
    cursor: "pointer",
    width: "100%",
    textAlign: "left",
    borderTop: "none",
    borderRight: "none",
    borderBottom: "none",
    color: "inherit",
    font: "inherit",
  },
  selected: {
    outline: `1px solid ${tokens.colorBrandStroke1}`,
  },
  detail: {
    padding: "14px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "10px",
  },
  detailHeader: {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    gap: "8px",
  },
  team: {
    display: "flex",
    flexDirection: "column",
    gap: "4px",
  },
  playerRow: {
    display: "grid",
    gridTemplateColumns: "28px 18px 18px 1fr auto",
    gap: "6px",
    alignItems: "center",
    padding: "4px 6px",
    borderRadius: "6px",
    background: tokens.colorNeutralBackground3,
  },
  own: {
    border: `1px solid ${tokens.colorBrandStroke1}`,
  },
  items: {
    display: "flex",
    gap: "2px",
  },
  tftUnits: {
    display: "flex",
    gap: "4px",
    flexWrap: "wrap",
  },
  playerLink: {
    minWidth: 0,
    padding: 0,
    border: "none",
    background: "transparent",
    color: "inherit",
    font: "inherit",
    textAlign: "left",
    cursor: "pointer",
    display: "grid",
    gap: "1px",
  },
  muted: {
    color: tokens.colorNeutralForeground3,
  },
  empty: {
    padding: "48px 12px",
    textAlign: "center",
    display: "grid",
    gap: "10px",
    justifyItems: "center",
  },
});

const championIcon = (id: number) =>
  `/lol-game-data/assets/v1/champion-icons/${id}.png`;

function matchesFilter(match: MatchSummary, filter: string): boolean {
  return filter === "all" || String(match.queueId) === filter;
}

export interface SearchRequest {
  query: string;
  gameId?: number;
  nonce: number;
}

export default function SearchPage({
  connected,
  active = true,
  request,
  onRequestHandled,
}: {
  connected: boolean;
  active?: boolean;
  request?: SearchRequest | null;
  onRequestHandled?: () => void;
}) {
  const styles = useStyles();
  const [query, setQuery] = useState("");
  const [player, setPlayer] = useState<PlayerPayload | null>(null);
  const [catalog, setCatalog] = useState<CatalogData>({});
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("all");
  const [extra, setExtra] = useState<MatchSummary[]>([]);
  const [hasMore, setHasMore] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [selected, setSelected] = useState<GameDetail | null>(null);
  const [tftSelected, setTftSelected] = useState<TftGameSummary | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [history, setHistory] = useState("");
  const searchSeq = useRef(0);
  const detailSeq = useRef(0);

  useEffect(() => {
    let disposed = false;
    void invoke<{ other?: { searchHistory?: string } }>("config_get")
      .then((config) => {
        if (!disposed) setHistory(config.other?.searchHistory ?? "");
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, []);

  const puuid = player?.summoner.puuid;

  const matches = useMemo(() => {
    const seen = new Set<number>();
    const merged: MatchSummary[] = [];
    for (const match of [...(player?.matches ?? []), ...extra]) {
      if (seen.has(match.gameId)) continue;
      seen.add(match.gameId);
      merged.push(match);
    }
    return merged;
  }, [player, extra]);

  const queues = useMemo(() => {
    const map = new Map<number, string>();
    for (const match of matches) {
      if (match.queueId > 0) map.set(match.queueId, match.queueName);
    }
    return [...map.entries()];
  }, [matches]);

  const visible = matches.filter((match) => matchesFilter(match, filter));
  const solo = player?.ranked?.queueMap?.RANKED_SOLO_5x5;
  const flex = player?.ranked?.queueMap?.RANKED_FLEX_SR;
  const summonerName =
    player?.summoner.gameName ?? player?.summoner.displayName ?? "";

  async function loadCatalog() {
    if (Object.keys(catalog).length > 0) return;
    try {
      setCatalog(await invoke<CatalogData>("catalog_get"));
    } catch {
      setCatalog({});
    }
  }

  async function runSearch(text: string, gameId?: number) {
    if (!connected || !text) return;
    const seq = ++searchSeq.current;
    setLoading(true);
    setError("");
    setSelected(null);
    setTftSelected(null);
    setExtra([]);
    setHasMore(true);
    setFilter("all");
    void loadCatalog();
    try {
      const result = await invoke<PlayerPayload>("player_overview", {
        query: text,
      });
      if (seq !== searchSeq.current) return;
      setPlayer(result);
      setHistory(text);
      void invoke("config_patch", {
        patch: { other: { searchHistory: text } },
      }).catch(() => undefined);
      if (gameId) {
        const token = ++detailSeq.current;
        setDetailLoading(true);
        try {
          const detail = await invoke<GameDetail>("game_detail", { gameId });
          if (seq === searchSeq.current && token === detailSeq.current) {
            setSelected(detail);
          }
        } catch (cause) {
          try {
            const tft = await invoke<{ games?: TftGameSummary[] }>(
              "tft_recent",
              { count: 40 },
            );
            if (seq !== searchSeq.current || token !== detailSeq.current) {
              return;
            }
            const game = (tft.games ?? []).find(
              (entry) => entry.gameId === gameId,
            );
            if (game) {
              setTftSelected(game);
            } else {
              setError(
                friendlyError(cause, "读取对局详情失败，请稍后重试。"),
              );
            }
          } catch {
            if (seq === searchSeq.current && token === detailSeq.current) {
              setError(
                friendlyError(cause, "读取对局详情失败，请稍后重试。"),
              );
            }
          }
        } finally {
          if (seq === searchSeq.current && token === detailSeq.current) {
            setDetailLoading(false);
          }
        }
      }
    } catch (cause) {
      if (seq !== searchSeq.current) return;
      setPlayer(null);
      setError(
        String(cause).includes("SummonerNotFound")
          ? "未找到该召唤师，请检查名称后重试。"
          : friendlyError(cause, "加载失败，请稍后重试。"),
      );
    } finally {
      if (seq === searchSeq.current) {
        setLoading(false);
      }
    }
  }

  function search() {
    const text = query.trim();
    if (text) void runSearch(text);
  }

  const requestNonce = request?.nonce;
  useEffect(() => {
    if (!request || !request.query) return;
    onRequestHandled?.();
    if (!active) return;
    setQuery(request.query);
    void runSearch(request.query, request.gameId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestNonce, active]);

  function focusPlayer(
    player: {
      puuid?: string;
      gameName?: string;
      tagLine?: string;
      summonerName?: string;
    },
    gameId?: number,
  ) {
    const target =
      player.puuid ||
      (player.gameName
        ? `${player.gameName}${player.tagLine ? `#${player.tagLine}` : ""}`
        : player.summonerName);
    if (!target) return;
    setQuery(target);
    void runSearch(target, gameId ?? selected?.gameId);
  }

  async function loadMore() {
    if (!puuid || loadingMore) return;
    setLoadingMore(true);
    try {
      const begin = matches.length;
      const page = await invoke<{ matches: MatchSummary[]; hasMore: boolean }>(
        "matches_page",
        { puuid, begin, end: begin + 9 },
      );
      setExtra((previous) => [...previous, ...page.matches]);
      setHasMore(page.hasMore && page.matches.length > 0);
    } catch (cause) {
      setError(friendlyError(cause, "加载更多失败，请稍后重试。"));
    } finally {
      setLoadingMore(false);
    }
  }

  async function openGame(match: MatchSummary) {
    const token = ++detailSeq.current;
    setSelected(null);
    setTftSelected(null);
    setError("");
    setDetailLoading(true);
    try {
      const detail = await invoke<GameDetail>("game_detail", {
        gameId: match.gameId,
      });
      if (token === detailSeq.current) setSelected(detail);
    } catch (cause) {
      if (token === detailSeq.current) {
        setError(friendlyError(cause, "读取对局详情失败，请稍后重试。"));
      }
    } finally {
      if (token === detailSeq.current) setDetailLoading(false);
    }
  }

  function queueName(detail: GameDetail): string {
    return (
      catalog.queues?.[String(detail.queueId)]?.name ??
      detail.gameMode ??
      "未知模式"
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.searchBar}>
        <Input
          style={{ flex: 1 }}
          value={query}
          placeholder={
            history
              ? `上次搜索：${history}`
              : "召唤师名称#TAG 或 PUUID"
          }
          onChange={(_, data) => setQuery(data.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void search();
          }}
        />
        <Button
          appearance="primary"
          disabled={!connected}
          onClick={() => void search()}
        >
          {loading ? "搜索中…" : "搜索"}
        </Button>
      </div>

      {!connected && (
        <MessageBar intent="warning">
          <MessageBarBody>请先启动并连接英雄联盟客户端。</MessageBarBody>
        </MessageBar>
      )}
      {error && (
        <MessageBar intent="error">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}

      {!player && (
        <div className={styles.empty}>
          <Title3>搜索召唤师</Title3>
          <Caption1 className={styles.muted}>
            输入名称或 PUUID 查看战绩。名称需与客户端内一致。
          </Caption1>
        </div>
      )}

      {player && (
        <>
          <div className={styles.header}>
            <AssetImage
              path={
                player.summoner.profileIconId
                  ? `/lol-game-data/assets/v1/profile-icons/${player.summoner.profileIconId}.jpg`
                  : undefined
              }
              size={64}
              radius={32}
            />
            <div>
              <Title3>
                {summonerName}
                {player.summoner.tagLine ? `#${player.summoner.tagLine}` : ""}
              </Title3>
              <Caption1 className={styles.muted}>
                等级 {player.summoner.summonerLevel ?? "-"}
              </Caption1>
            </div>
          </div>

          <div className={styles.rankRow}>
            <div className={styles.card}>
              <Caption1 className={styles.muted}>单双排</Caption1>
              <Subtitle1>{rankText(solo)}</Subtitle1>
              <Caption1>
                {solo
                  ? `${solo.leaguePoints ?? 0} LP · ${winRate(solo)}`
                  : "本赛季暂无数据"}
              </Caption1>
            </div>
            <div className={styles.card}>
              <Caption1 className={styles.muted}>灵活组排</Caption1>
              <Subtitle1>{rankText(flex)}</Subtitle1>
              <Caption1>
                {flex
                  ? `${flex.leaguePoints ?? 0} LP · ${winRate(flex)}`
                  : "本赛季暂无数据"}
              </Caption1>
            </div>
          </div>

          <div className={styles.grid}>
            <div className={styles.column}>
              <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                <Subtitle1 style={{ flex: 1 }}>
                  对局记录（{visible.length}）
                </Subtitle1>
                <Dropdown
                  aria-label="筛选队列"
                  value={
                    filter === "all"
                      ? "全部"
                      : (queues.find(([id]) => String(id) === filter)?.[1] ??
                        "全部")
                  }
                  selectedOptions={[filter]}
                  onOptionSelect={(_, data) =>
                    setFilter(data.optionValue ?? "all")
                  }
                >
                  <Option value="all">全部</Option>
                  {queues.map(([id, name]) => (
                    <Option key={id} value={String(id)}>
                      {name}
                    </Option>
                  ))}
                </Dropdown>
              </div>

              {visible.map((match) => (
                <button
                  key={match.gameId}
                  type="button"
                  className={`${styles.matchRow} ${
                    selected?.gameId === match.gameId ? styles.selected : ""
                  }`}
                  style={{
                    borderLeftColor: match.win
                      ? tokens.colorPaletteGreenBorder1
                      : tokens.colorPaletteRedBorder1,
                  }}
                  onClick={() => void openGame(match)}
                >
                  <AssetImage
                    path={championIcon(match.championId)}
                    size={40}
                    title={match.championName}
                  />
                  <span style={{ minWidth: 0 }}>
                    <Text weight="semibold">{match.championName}</Text>
                    <br />
                    <Caption1 className={styles.muted}>
                      {match.queueName} · {formatTime(match.gameCreation)} ·{" "}
                      {formatDuration(match.gameDuration)}
                    </Caption1>
                  </span>
                  <span style={{ textAlign: "right" }}>
                    <Badge
                      appearance="tint"
                      color={match.win ? "success" : "danger"}
                    >
                      {match.win ? "胜利" : "失败"}
                    </Badge>
                    <br />
                    <Text weight="semibold">
                      {match.kills}/{match.deaths}/{match.assists}
                    </Text>
                  </span>
                </button>
              ))}

              {visible.length === 0 && (
                <Caption1 className={styles.muted}>没有符合筛选的对局。</Caption1>
              )}

              {hasMore && filter === "all" && (
                <Button
                  appearance="subtle"
                  disabled={loadingMore}
                  onClick={() => void loadMore()}
                >
                  {loadingMore ? "加载中…" : "加载更多"}
                </Button>
              )}
            </div>

            <div className={styles.column}>
              {detailLoading && <Spinner label="读取对局详情…" />}
              {!detailLoading && !selected && !tftSelected && (
                <div className={styles.detail}>
                  <Caption1 className={styles.muted}>
                    点击左侧对局查看双方阵容、装备与符文。
                  </Caption1>
                </div>
              )}
              {tftSelected && !detailLoading && (
                <div className={styles.detail}>
                  <div className={styles.detailHeader}>
                    <div>
                      <Subtitle1>云顶之弈</Subtitle1>
                      <Caption1 className={styles.muted}>
                        {formatTime(tftSelected.gameCreation)} ·{" "}
                        {formatDuration(tftSelected.duration)}
                      </Caption1>
                    </div>
                    <Button
                      appearance="subtle"
                      size="small"
                      onClick={() => setTftSelected(null)}
                    >
                      收起
                    </Button>
                  </div>
                  <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
                    <Badge appearance="tint" color={tftSelected.placement <= 4 ? "success" : "danger"}>
                      第 {tftSelected.placement} 名
                    </Badge>
                    <Caption1>等级 {tftSelected.level}</Caption1>
                    <Caption1 className={styles.muted}>
                      Set {tftSelected.setNumber}
                    </Caption1>
                  </div>
                  {Boolean(tftSelected.augments?.length) && (
                    <Caption1 className={styles.muted}>
                      强化符文：
                      {(tftSelected.augments ?? [])
                        .map((augment) => augment.name || augment.id)
                        .join(" / ")}
                    </Caption1>
                  )}
                  <div className={styles.tftUnits}>
                    {(tftSelected.units ?? []).map((unit, index) => (
                      <AssetImage
                        key={`${tftSelected.gameId}-${index}`}
                        path={
                          unit.championId > 0
                            ? championIcon(unit.championId)
                            : undefined
                        }
                        size={30}
                        radius={4}
                        title={`${unit.championName} · ${unit.tier} 星`}
                      />
                    ))}
                  </div>
                </div>
              )}
              {selected && !detailLoading && (
                <GameDetailView
                  detail={selected}
                  catalog={catalog}
                  puuid={puuid}
                  queueName={queueName(selected)}
                  styles={styles}
                  onClose={() => {
                    detailSeq.current += 1;
                    setSelected(null);
                  }}
                  onPlayerClick={(player) =>
                    focusPlayer(player, selected.gameId)
                  }
                />
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

type Styles = ReturnType<typeof useStyles>;

function GameDetailView({
  detail,
  catalog,
  puuid,
  queueName,
  styles,
  onClose,
  onPlayerClick,
}: {
  detail: GameDetail;
  catalog: CatalogData;
  puuid?: string;
  queueName: string;
  styles: Styles;
  onClose: () => void;
  onPlayerClick: (player: {
    puuid?: string;
    gameName?: string;
    tagLine?: string;
    summonerName?: string;
  }) => void;
}) {
  const teams = detail.teams ?? [];
  const participants = detail.participants ?? [];
  const identities = detail.participantIdentities ?? [];
  const ownParticipant = participants.find((participant) => {
    const identity = identities.find(
      (entry) => entry.participantId === participant.participantId,
    );
    return puuid !== undefined && identity?.player?.puuid === puuid;
  });
  const blueWin = teams.find((team) => team.teamId === 100)?.win;
  const result =
    ownParticipant?.stats.win ??
    (blueWin === undefined ? null : blueWin);

  return (
    <div className={styles.detail}>
      <div className={styles.detailHeader}>
        <div>
          <Subtitle1>{queueName}</Subtitle1>
          <Caption1 className={styles.muted}>
            {formatTime(detail.gameCreation)} ·{" "}
            {formatDuration(detail.gameDuration)}
          </Caption1>
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          {result !== null && (
            <Badge
              appearance="tint"
              color={result ? "success" : "danger"}
            >
              {result ? "胜利" : "失败"}
            </Badge>
          )}
          <Button appearance="subtle" size="small" onClick={onClose}>
            收起
          </Button>
        </div>
      </div>

      {[100, 200].map((teamId) => {
        const team = teams.find((entry) => entry.teamId === teamId);
        const rows = participants.filter(
          (participant) => participant.teamId === teamId,
        );
        return (
          <div className={styles.team} key={teamId}>
            <Caption1 className={styles.muted}>
              {teamId === 100 ? "蓝色方" : "红色方"}
              {team ? (team.win ? " · 胜利" : " · 失败") : ""}
            </Caption1>
            {rows.map((row) => {
              const identity = identities.find(
                (entry) => entry.participantId === row.participantId,
              );
              const player = identity?.player;
              const name =
                player?.gameName && player.tagLine
                  ? `${player.gameName}#${player.tagLine}`
                  : (player?.summonerName ?? "未知玩家");
              const own = puuid !== undefined && player?.puuid === puuid;
              const stats = row.stats;
              const cs =
                (stats.totalMinionsKilled ?? 0) +
                (stats.neutralMinionsKilled ?? 0);
              const items = [0, 1, 2, 3, 4, 5, 6]
                .map((index) => Number(stats[`item${index}`] ?? 0))
                .filter((id) => id > 0);
              const spell1 = catalog.spells?.[String(row.spell1Id)]?.iconPath;
              const spell2 = catalog.spells?.[String(row.spell2Id)]?.iconPath;
              const perk = catalog.runes?.[String(stats.perk0 ?? 0)]?.icon;
              return (
                <div
                  key={row.participantId}
                  className={`${styles.playerRow} ${own ? styles.own : ""}`}
                >
                  <AssetImage
                    path={championIcon(row.championId)}
                    size={28}
                    title={catalog.champions?.[String(row.championId)]}
                  />
                  <div style={{ display: "grid", gap: 1 }}>
                    <AssetImage path={spell1} size={18} radius={3} />
                    <AssetImage path={spell2} size={18} radius={3} />
                  </div>
                  <AssetImage path={perk} size={18} radius={9} />
                  <button
                    type="button"
                    className={styles.playerLink}
                    title={`查看 ${name} 的战绩`}
                    onClick={() => player && onPlayerClick(player)}
                  >
                    <Text weight={own ? "semibold" : "regular"} truncate>
                      {name}
                    </Text>
                    <Caption1 className={styles.muted}>
                      {catalog.champions?.[String(row.championId)] ?? ""} ·{" "}
                      {stats.kills ?? 0}/{stats.deaths ?? 0}/
                      {stats.assists ?? 0} · {cs} CS
                    </Caption1>
                  </button>
                  <div className={styles.items}>
                    {items.map((itemId, index) => (
                      <AssetImage
                        key={`${row.participantId}-${index}`}
                        path={catalog.items?.[String(itemId)]?.iconPath}
                        size={22}
                        radius={4}
                      />
                    ))}
                  </div>
                </div>
              );
            })}
            {team?.bans && team.bans.some((ban) => ban.championId > 0) && (
              <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
                <Caption1 className={styles.muted}>禁用</Caption1>
                {team.bans
                  .filter((ban) => ban.championId > 0)
                  .map((ban, index) => (
                    <AssetImage
                      key={`${teamId}-ban-${index}`}
                      path={championIcon(ban.championId)}
                      size={20}
                      radius={4}
                      title={catalog.champions?.[String(ban.championId)]}
                    />
                  ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}