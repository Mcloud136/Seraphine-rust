export function formatDuration(seconds: number): string {
  if (!seconds) return "--:--";
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}

export function formatTime(epochMs: number): string {
  if (!epochMs) return "";
  return new Date(epochMs).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export interface RankedEntry {
  tier?: string;
  division?: string;
  leaguePoints?: number;
  wins?: number;
  losses?: number;
}

export function rankText(entry: RankedEntry | undefined): string {
  if (!entry || !entry.tier) return "未定级";
  return `${entry.tier} ${entry.division ?? ""}`.trim();
}

export function winRate(entry: RankedEntry | undefined): string {
  if (!entry) return "";
  const wins = entry.wins ?? 0;
  const losses = entry.losses ?? 0;
  const total = wins + losses;
  if (!total) return "";
  return `${Math.round((wins / total) * 100)}% · ${wins}胜 ${losses}负`;
}