import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Badge,
  Button,
  Caption1,
  Spinner,
  Text,
  Title3,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { ArrowClockwiseRegular } from "@fluentui/react-icons";
import { AssetImage } from "./AssetImage";
import { formatDuration, formatTime } from "./format";

interface TftUnit {
  characterId: string;
  alias: string;
  championId: number;
  championName: string;
  tier: number;
  rarity: number;
}

interface TftAugment {
  id: number;
  name: string;
}

interface TftGame {
  gameId: number;
  placement: number;
  win: boolean;
  level: number;
  duration: number;
  gameCreation: number;
  setNumber: number;
  units: TftUnit[];
  augments: TftAugment[];
}

interface TftPayload {
  puuid: string;
  wins: number;
  losses: number;
  averagePlacement: number;
  games: TftGame[];
}

const useStyles = makeStyles({
  page: { display: "flex", flexDirection: "column", gap: "14px" },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "12px",
  },
  summary: { display: "flex", gap: "18px", flexWrap: "wrap" },
  list: { display: "flex", flexDirection: "column", gap: "8px" },
  card: {
    padding: "10px 12px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    gap: "6px",
  },
  cardTop: {
    display: "flex",
    gap: "10px",
    alignItems: "center",
    flexWrap: "wrap",
  },
  units: { display: "flex", gap: "3px", flexWrap: "wrap" },
  unit: {
    borderRadius: "5px",
    border: "2px solid transparent",
  },
  muted: { color: tokens.colorNeutralForeground3 },
  empty: {
    padding: "48px 12px",
    textAlign: "center",
    display: "grid",
    gap: "10px",
    justifyItems: "center",
  },
});

function placementColor(placement: number): "success" | "danger" | "warning" {
  if (placement === 1) return "warning";
  if (placement > 0 && placement <= 4) return "success";
  return "danger";
}

function tierColor(tier: number): string {
  switch (tier) {
    case 1:
      return "#9aa4af";
    case 2:
      return "#2fb67c";
    case 3:
      return "#3d8fd8";
    case 4:
      return "#b45cf0";
    default:
      return "transparent";
  }
}

export default function TftPage({ connected }: { connected: boolean }) {
  const styles = useStyles();
  const [data, setData] = useState<TftPayload | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    if (!connected) {
      setData(null);
      return;
    }
    setLoading(true);
    try {
      const result = await invoke<TftPayload>("tft_recent", { count: 20 });
      setData(result);
      setError("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, [connected]);

  useEffect(() => {
    void load();
  }, [load]);

  const games = data?.games ?? [];
  const total = (data?.wins ?? 0) + (data?.losses ?? 0);
  const top4 = total > 0 ? Math.round(((data?.wins ?? 0) / total) * 100) : 0;

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <div>
          <Title3>TFT 战绩</Title3>
          <div className={styles.summary}>
            <Caption1>
              最近 {games.length} 场 · 前四率 {top4}%（{data?.wins ?? 0}/
              {total}）
            </Caption1>
            <Caption1 className={styles.muted}>
              平均排名 {data?.averagePlacement ?? 0}
            </Caption1>
          </div>
        </div>
        <Button
          appearance="subtle"
          icon={<ArrowClockwiseRegular />}
          disabled={!connected || loading}
          onClick={() => void load()}
        >
          刷新
        </Button>
      </div>

      {error && <Caption1 className={styles.muted}>{error}</Caption1>}
      {loading && !data && <Spinner label="读取 TFT 战绩…" />}
      {!loading && connected && games.length === 0 && (
        <div className={styles.empty}>
          <Title3>暂无 TFT 战绩</Title3>
          <Caption1 className={styles.muted}>
            完成一局云顶之弈后，这里会显示排名、羁绊与强化符文。
          </Caption1>
        </div>
      )}
      {!connected && (
        <div className={styles.empty}>
          <Title3>等待英雄联盟客户端</Title3>
        </div>
      )}

      <div className={styles.list}>
        {games.map((game) => (
          <div className={styles.card} key={game.gameId}>
            <div className={styles.cardTop}>
              <Badge
                appearance="tint"
                color={placementColor(game.placement)}
              >
                第 {game.placement} 名
              </Badge>
              <Text weight="semibold">等级 {game.level}</Text>
              <Caption1 className={styles.muted}>
                Set {game.setNumber} · {formatDuration(game.duration)} ·{" "}
                {formatTime(game.gameCreation)}
              </Caption1>
              {game.augments.length > 0 && (
                <Caption1 className={styles.muted}>
                  强化：{game.augments.map((a) => a.name || a.id).join(" / ")}
                </Caption1>
              )}
            </div>
            <div className={styles.units}>
              {game.units.map((unit, index) => (
                <div
                  key={`${game.gameId}-${index}`}
                  className={styles.unit}
                  style={{ borderColor: tierColor(unit.tier) }}
                  title={`${unit.championName} · ${unit.tier} 星`}
                >
                  <AssetImage
                    path={
                      unit.championId > 0
                        ? `/lol-game-data/assets/v1/champion-icons/${unit.championId}.png`
                        : undefined
                    }
                    size={30}
                    radius={3}
                  />
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}