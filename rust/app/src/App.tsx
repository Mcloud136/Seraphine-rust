import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Badge,
  Button,
  Caption1,
  Divider,
  FluentProvider,
  MessageBar,
  MessageBarBody,
  Spinner,
  Subtitle1,
  Tab,
  TabList,
  Text,
  Title3,
  makeStyles,
  tokens,
  webDarkTheme,
  webLightTheme,
} from "@fluentui/react-components";
import {
  ChevronLeftRegular,
  ChevronRightRegular,
  ArrowClockwiseRegular,
  DismissRegular,
  GamesRegular,
  HomeRegular,
  OptionsRegular,
  PersonRegular,
  SearchRegular,
  SettingsRegular,
  SquareRegular,
  SubtractRegular,
  TrophyRegular,
  WrenchRegular,
} from "@fluentui/react-icons";
import SearchPage, { type SearchRequest } from "./SearchPage";
import HomePage from "./HomePage";
import GameFunctionsPage from "./GameFunctionsPage";
import GameInfoPage from "./GameInfoPage";
import TftPage from "./TftPage";
import OpggPage from "./OpggPage";
import SettingsPage from "./SettingsPage";
import AuxiliaryPage from "./AuxiliaryPage";
import type { AppConfig } from "./config";

interface SessionStatus {
  connected: boolean;
  pid: number;
  port: number;
  server: string;
  epoch: number;
  websocketReceived: number;
  websocketDropped: number;
  websocketReconnects: number;
}

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

interface CareerPayload {
  summoner: Summoner;
  ranked: { queueMap?: Record<string, RankedEntry> } | null;
  matches: MatchSummary[];
}

type SessionEventPayload =
  | { type: "connected"; info: { pid: number } }
  | { type: "disconnected"; reason: string }
  | { type: "lcu"; uri: string }
  | { type: "error"; message: string };

const useStyles = makeStyles({
  root: {
    height: "100vh",
    display: "grid",
    gridTemplateRows: "36px 1fr",
    color: tokens.colorNeutralForeground1,
  },
  titlebar: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    paddingLeft: "14px",
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  brand: {
    display: "flex",
    alignItems: "baseline",
    gap: "8px",
  },
  brandTitle: {
    fontSize: "12px",
    fontWeight: 700,
    letterSpacing: "1.2px",
  },
  titlebarRight: {
    display: "flex",
    alignItems: "center",
  },
  connection: {
    display: "flex",
    alignItems: "center",
    gap: "6px",
    marginRight: "12px",
  },
  statusDot: {
    width: "8px",
    height: "8px",
    borderRadius: "50%",
  },
  controlButton: {
    minWidth: "46px",
    height: "36px",
    borderRadius: 0,
  },
  body: {
    display: "grid",
    gridTemplateColumns: "200px 1fr",
    minHeight: 0,
  },
  sidebar: {
    padding: "10px",
    borderRight: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "6px",
    minHeight: 0,
  },
  navSpacer: {
    flex: 1,
  },
  content: {
    padding: "24px 28px",
    overflowY: "auto",
    minHeight: 0,
  },
  header: {
    display: "flex",
    gap: "16px",
    alignItems: "center",
    marginBottom: "18px",
  },
  avatar: {
    width: "64px",
    height: "64px",
    borderRadius: "50%",
    background: tokens.colorNeutralBackground3,
    display: "grid",
    placeItems: "center",
    fontSize: "24px",
    fontWeight: 600,
    overflow: "hidden",
    flexShrink: 0,
  },
  rankRow: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))",
    gap: "12px",
    marginBottom: "18px",
  },
  card: {
    padding: "14px 16px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "4px",
  },
  sectionTitle: {
    margin: "8px 0 12px",
  },
  matchList: {
    display: "flex",
    flexDirection: "column",
    gap: "8px",
  },
  matchRow: {
    display: "grid",
    gridTemplateColumns: "40px 1fr auto",
    gap: "12px",
    alignItems: "center",
    padding: "10px 12px",
    borderRadius: "8px",
    background: tokens.colorNeutralBackground2,
    borderLeftWidth: "3px",
    borderLeftStyle: "solid",
    cursor: "pointer",
    width: "100%",
    textAlign: "left",
    color: "inherit",
    font: "inherit",
    borderTop: "none",
    borderRight: "none",
    borderBottom: "none",
  },
  toggleButton: {
    alignSelf: "flex-start",
    minWidth: "32px",
  },
  actionsRow: {
    display: "flex",
    gap: "6px",
    alignItems: "center",
    justifyContent: "flex-start",
    paddingLeft: "2px",
    paddingBottom: "4px",
  },
  actionRound: {
    minWidth: "34px",
    width: "34px",
    height: "34px",
  },
  championIcon: {
    width: "40px",
    height: "40px",
    borderRadius: "6px",
    objectFit: "cover",
    background: tokens.colorNeutralBackground3,
  },
  kda: {
    fontVariantNumeric: "tabular-nums",
    fontWeight: 600,
  },
  muted: {
    color: tokens.colorNeutralForeground3,
  },
  empty: {
    padding: "48px 12px",
    textAlign: "center",
    display: "grid",
    gap: "12px",
    justifyItems: "center",
  },
});

function connectionLabel(status: SessionStatus | null): string {
  if (!status) return "检测中";
  if (!status.connected) return "客户端未连接";
  return `已连接 · ${status.server || "未知区服"}`;
}

function formatDuration(seconds: number): string {
  if (!seconds) return "--:--";
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

function formatTime(epochMs: number): string {
  if (!epochMs) return "";
  return new Date(epochMs).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function rankText(entry: RankedEntry | undefined): string {
  if (!entry || !entry.tier) return "未定级";
  return `${entry.tier} ${entry.division ?? ""}`.trim();
}

function winRate(entry: RankedEntry | undefined): string {
  if (!entry) return "";
  const wins = entry.wins ?? 0;
  const losses = entry.losses ?? 0;
  const total = wins + losses;
  if (!total) return "";
  return `${Math.round((wins / total) * 100)}% · ${wins}胜 ${losses}负`;
}

export default function App() {
  const styles = useStyles();
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<SessionStatus | null>(null);
  const [career, setCareer] = useState<CareerPayload | null>(null);
  const [icons, setIcons] = useState<Record<number, string>>({});
  const [profileIcon, setProfileIcon] = useState("");
  const [error, setError] = useState("");
  const [tab, setTab] = useState("home");
  const [connecting, setConnecting] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [searchRequest, setSearchRequest] = useState<SearchRequest | null>(
    null,
  );

  const openSearch = useCallback(
    (request: { query: string; gameId?: number }) => {
      setSearchRequest({ ...request, nonce: Date.now() });
      setTab("search");
    },
    [],
  );

  const dark = config?.theme.themeMode?.toLowerCase() === "dark";

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<SessionStatus>("session_status");
      setStatus(next);
      if (next.connected) {
        const data = await invoke<CareerPayload>("career_overview");
        setCareer(data);
      } else {
        setCareer(null);
      }
      setError("");
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  const connect = useCallback(async () => {
    setConnecting(true);
    try {
      await invoke("session_connect", { pid: null });
      await refresh();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setConnecting(false);
    }
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    invoke<AppConfig>("config_get")
      .then((value) => {
        if (!disposed) setConfig(value);
      })
      .catch((cause) => setError(String(cause)));

    const unlisten = listen<SessionEventPayload>("session://event", (event) => {
      const payload = event.payload;
      if (payload.type === "connected") {
        void refresh();
      } else if (payload.type === "disconnected") {
        setCareer(null);
        setStatus(null);
      } else if (payload.type === "error") {
        setError(payload.message);
      }
    });

    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
      void unlisten.then((stop) => stop());
    };
  }, [refresh]);

  useEffect(() => {
    if (!career) return;
    let disposed = false;
    for (const match of career.matches) {
      if (match.championId <= 0) continue;
      if (icons[match.championId]) continue;
      void invoke<string>("asset", {
        path: `/lol-game-data/assets/v1/champion-icons/${match.championId}.png`,
      })
        .then((url) => {
          if (!disposed) {
            setIcons((previous) => ({ ...previous, [match.championId]: url }));
          }
        })
        .catch(() => undefined);
    }
    return () => {
      disposed = true;
    };
  }, [career, icons]);

  useEffect(() => {
    const iconId = career?.summoner.profileIconId;
    if (!iconId) {
      setProfileIcon("");
      return;
    }
    let disposed = false;
    void invoke<string>("asset", {
      path: `/lol-game-data/assets/v1/profile-icons/${iconId}.jpg`,
    })
      .then((url) => {
        if (!disposed) setProfileIcon(url);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, [career?.summoner.profileIconId]);

  const solo = career?.ranked?.queueMap?.RANKED_SOLO_5x5;
  const flex = career?.ranked?.queueMap?.RANKED_FLEX_SR;
  const summonerName =
    career?.summoner.gameName ??
    career?.summoner.displayName ??
    "未获取召唤师";
  const ownQuery =
    career?.summoner.puuid ||
    (career
      ? `${summonerName}${career.summoner.tagLine ? `#${career.summoner.tagLine}` : ""}`
      : "");

  return (
    <FluentProvider
      theme={dark ? webDarkTheme : webLightTheme}
      style={{ height: "100%" }}
    >
      <div
        className={styles.root}
        style={{
          backgroundColor: dark
            ? "rgba(30, 30, 30, 0.80)"
            : "rgba(247, 247, 247, 0.80)",
        }}
      >
        <div className={styles.titlebar} data-tauri-drag-region>
          <div className={styles.brand}>
            <Text className={styles.brandTitle}>SERAPHINE</Text>
            <Caption1 className={styles.muted}>Rust</Caption1>
          </div>
          <div className={styles.titlebarRight}>
            <div className={styles.connection}>
              <span
                className={styles.statusDot}
                style={{
                  background: status?.connected
                    ? tokens.colorPaletteGreenBackground3
                    : tokens.colorPaletteRedBackground3,
                }}
              />
              <Caption1>{connectionLabel(status)}</Caption1>
            </div>
            <Button
              appearance="subtle"
              className={styles.controlButton}
              icon={<SubtractRegular />}
              aria-label="最小化"
              onClick={() => void getCurrentWindow().minimize()}
            />
            <Button
              appearance="subtle"
              className={styles.controlButton}
              icon={<SquareRegular />}
              aria-label="最大化"
              onClick={() => void getCurrentWindow().toggleMaximize()}
            />
            <Button
              appearance="subtle"
              className={styles.controlButton}
              icon={<DismissRegular />}
              aria-label="关闭"
              onClick={() => void getCurrentWindow().close()}
            />
          </div>
        </div>

        <div
          className={styles.body}
          style={{
            gridTemplateColumns: collapsed ? "64px 1fr" : "128px 1fr",
          }}
        >
          <nav className={styles.sidebar}>
            <Button
              appearance="subtle"
              className={styles.toggleButton}
              icon={
                collapsed ? <ChevronRightRegular /> : <ChevronLeftRegular />
              }
              aria-label={collapsed ? "展开导航" : "收起导航"}
              onClick={() => setCollapsed((value) => !value)}
            />
            <TabList
              vertical
              selectedValue={tab}
              onTabSelect={(_, data) => setTab(String(data.value))}
            >
              <Tab
                value="home"
                icon={<HomeRegular />}
                title="首页"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "首页"}
              </Tab>
              <Tab
                value="career"
                icon={<PersonRegular />}
                title="个人信息"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "个人信息"}
              </Tab>
              <Tab
                value="search"
                icon={<SearchRegular />}
                title="搜索信息"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "搜索信息"}
              </Tab>
              <Tab
                value="gameinfo"
                icon={<GamesRegular />}
                title="对局信息"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "对局信息"}
              </Tab>
              <Tab
                value="functions"
                icon={<OptionsRegular />}
                title="游戏功能"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "游戏功能"}
              </Tab>
              <Tab
                value="opgg"
                icon={<TrophyRegular />}
                title="OP.GG"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "OP.GG"}
              </Tab>
              <Tab
                value="tft"
                icon={<GamesRegular />}
                title="云顶之弈"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "云顶之弈"}
              </Tab>
              <Tab
                value="auxiliary"
                icon={<WrenchRegular />}
                title="辅助工具"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "辅助工具"}
              </Tab>
              <Tab
                value="settings"
                icon={<SettingsRegular />}
                title="设置"
                style={collapsed ? { minWidth: 0 } : undefined}
              >
                {!collapsed && "设置"}
              </Tab>
            </TabList>
            <div className={styles.navSpacer} />
            <div
              className={styles.actionsRow}
              style={{ flexDirection: collapsed ? "column" : "row" }}
            >
              <Button
                appearance="outline"
                shape="circular"
                className={styles.actionRound}
                icon={<WrenchRegular />}
                title="修复客户端窗口"
                aria-label="修复客户端窗口"
                onClick={() => {
                  void invoke("fix_client_window").catch((cause) =>
                    setError(String(cause)),
                  );
                }}
              />
              <Button
                appearance="outline"
                shape="circular"
                className={styles.actionRound}
                icon={<ArrowClockwiseRegular />}
                title="重启客户端"
                aria-label="重启客户端"
                onClick={() => {
                  void invoke("write_lcu", {
                    operationId: crypto.randomUUID(),
                    method: "POST",
                    path: "/riotclient/kill-and-restart-ux",
                    data: null,
                  }).catch((cause) => setError(String(cause)));
                }}
              />
            </div>
            {!collapsed && (
              <>
                <Divider />
                {status?.connected ? (
                  <>
                    <Caption1 className={styles.muted}>
                      PID {status.pid} · 事件 {status.websocketReceived}
                    </Caption1>
                    <Caption1 className={styles.muted}>
                      epoch {status.epoch} · 重连 {status.websocketReconnects}
                    </Caption1>
                  </>
                ) : (
                  <Caption1 className={styles.muted}>
                    等待英雄联盟客户端
                  </Caption1>
                )}
              </>
            )}
          </nav>

          <main className={styles.content}>
            {error && (
              <MessageBar intent="error" style={{ marginBottom: 16 }}>
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}

            {tab === "home" && <HomePage />}

            {tab === "career" && (
              <>
                {!status?.connected ? (
                  <div className={styles.empty}>
                    <Title3>等待英雄联盟客户端</Title3>
                    <Caption1 className={styles.muted}>
                      启动客户端后会自动连接；也可以手动重试。
                    </Caption1>
                    <Button
                      appearance="primary"
                      disabled={connecting}
                      onClick={() => void connect()}
                    >
                      {connecting ? "连接中…" : "立即连接"}
                    </Button>
                  </div>
                ) : !career ? (
                  <div className={styles.empty}>
                    <Spinner label="读取召唤师数据…" />
                  </div>
                ) : (
                  <>
                    <header className={styles.header}>
                      <div className={styles.avatar}>
                        {profileIcon ? (
                          <img
                            src={profileIcon}
                            alt={summonerName}
                            style={{ width: "100%", height: "100%", objectFit: "cover" }}
                          />
                        ) : (
                          summonerName.slice(0, 1)
                        )}
                      </div>
                      <div>
                        <Title3>{summonerName}</Title3>
                        <Caption1 className={styles.muted}>
                          {career.summoner.tagLine
                            ? `#${career.summoner.tagLine} · `
                            : ""}
                          等级 {career.summoner.summonerLevel ?? "-"}
                        </Caption1>
                      </div>
                    </header>

                    <section className={styles.rankRow}>
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
                      <div className={styles.card}>
                        <Caption1 className={styles.muted}>最近对局</Caption1>
                        <Subtitle1>{career.matches.length} 场</Subtitle1>
                        <Caption1>
                          {career.matches.filter((match) => match.win).length} 胜
                        </Caption1>
                      </div>
                    </section>

                    <Subtitle1 className={styles.sectionTitle}>
                      对局记录
                    </Subtitle1>
                    <div className={styles.matchList}>
                      {career.matches.map((match) => (
                        <button
                          type="button"
                          key={match.gameId}
                          className={styles.matchRow}
                          title="在搜索中查看这一局"
                          disabled={!ownQuery}
                          onClick={() =>
                            openSearch({
                              query: ownQuery,
                              gameId: match.gameId,
                            })
                          }
                          style={{
                            borderLeftColor: match.win
                              ? tokens.colorPaletteGreenBorder1
                              : tokens.colorPaletteRedBorder1,
                          }}
                        >
                          {icons[match.championId] ? (
                            <img
                              className={styles.championIcon}
                              src={icons[match.championId]}
                              alt={match.championName}
                            />
                          ) : (
                            <div className={styles.championIcon} />
                          )}
                          <div>
                            <Text weight="semibold">
                              {match.championName}
                            </Text>
                            <Caption1 className={styles.muted}>
                              {" "}
                              · {match.queueName} · {formatTime(match.gameCreation)}
                            </Caption1>
                            <div>
                              <Caption1>
                                {formatDuration(match.gameDuration)}
                              </Caption1>
                            </div>
                          </div>
                          <div style={{ textAlign: "right" }}>
                            <Badge
                              appearance="tint"
                              color={match.win ? "success" : "danger"}
                            >
                              {match.win ? "胜利" : "失败"}
                            </Badge>
                            <div className={styles.kda}>
                              {match.kills} / {match.deaths} / {match.assists}
                            </div>
                          </div>
                        </button>
                      ))}
                      {career.matches.length === 0 && (
                        <Caption1 className={styles.muted}>
                          暂无对局记录
                        </Caption1>
                      )}
                    </div>
                  </>
                )}
              </>
            )}

            {tab === "search" && (
              <SearchPage
                connected={Boolean(status?.connected)}
                request={searchRequest}
              />
            )}

            {tab === "gameinfo" && (
              <GameInfoPage
                connected={Boolean(status?.connected)}
                onOpenSearch={openSearch}
              />
            )}

            {tab === "tft" && (
              <TftPage
                connected={Boolean(status?.connected)}
                onOpenMatch={(gameId, puuid) => {
                  const query = puuid || ownQuery;
                  if (query) openSearch({ query, gameId });
                }}
              />
            )}

            {tab === "functions" && config && (
              <GameFunctionsPage config={config} onSaved={setConfig} />
            )}

            {tab === "opgg" && (
              <OpggPage connected={Boolean(status?.connected)} />
            )}

            {tab === "settings" && config && (
              <SettingsPage config={config} onSaved={setConfig} />
            )}

            {tab === "auxiliary" && <AuxiliaryPage />}

            {tab !== "home" &&
              tab !== "career" &&
              tab !== "functions" &&
              tab !== "search" &&
              tab !== "gameinfo" &&
              tab !== "tft" &&
              tab !== "opgg" &&
              tab !== "settings" &&
              tab !== "auxiliary" && (
              <div className={styles.empty}>
                <Title3>该页面将在后续波次接入</Title3>
                <Caption1 className={styles.muted}>
                  当前切片已打通 Rust 会话、配置迁移与职业页真实数据。
                </Caption1>
              </div>
            )}
          </main>
        </div>
      </div>
    </FluentProvider>
  );
}