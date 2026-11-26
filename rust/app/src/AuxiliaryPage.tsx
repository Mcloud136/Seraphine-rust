import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
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

const useStyles = makeStyles({
  page: { display: "flex", flexDirection: "column", gap: "12px" },
  grid: {
    display: "block",
    columnWidth: "360px",
    columnGap: "12px",
  },
  card: {
    padding: "14px 16px",
    borderRadius: "10px",
    background: tokens.colorNeutralBackground2,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "inline-flex",
    flexDirection: "column",
    gap: "10px",
    minWidth: 0,
    width: "100%",
    maxWidth: "560px",
    breakInside: "avoid",
    marginBottom: "12px",
    verticalAlign: "top",
  },
  row: { display: "flex", gap: "8px", alignItems: "center" },
  muted: { color: tokens.colorNeutralForeground3 },
});

const AVAILABILITY = [
  { value: "online", label: "在线" },
  { value: "away", label: "离开" },
  { value: "dnd", label: "请勿打扰" },
  { value: "offline", label: "离线显示" },
];

const TIERS = [
  "UNRANKED",
  "IRON",
  "BRONZE",
  "SILVER",
  "GOLD",
  "PLATINUM",
  "EMERALD",
  "DIAMOND",
  "MASTER",
  "GRANDMASTER",
  "CHALLENGER",
];

const DIVISIONS = ["I", "II", "III", "IV"];

const DISPLAY_QUEUES = [
  { value: "RANKED_SOLO_5x5", label: "单双排" },
  { value: "RANKED_FLEX_SR", label: "灵活组排" },
];

export default function AuxiliaryPage() {
  const styles = useStyles();
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [statusMessage, setStatusMessage] = useState("");
  const [availability, setAvailability] = useState("online");
  const [profileIcon, setProfileIcon] = useState("");
  const [backgroundSkin, setBackgroundSkin] = useState("");
  const [spectateName, setSpectateName] = useState("");
  const [displayQueue, setDisplayQueue] = useState("RANKED_SOLO_5x5");
  const [displayTier, setDisplayTier] = useState("GOLD");
  const [displayDivision, setDisplayDivision] = useState("I");
  const [lockState, setLockState] = useState({ available: false, locked: false });

  useEffect(() => {
    void invoke<boolean>("writes_enabled")
      .then(setEnabled)
      .catch(() => setEnabled(false));
    void invoke<{ available: boolean; locked: boolean }>("lock_config_state")
      .then(setLockState)
      .catch(() => undefined);
  }, []);

  async function write(path: string, method: string, data?: unknown) {
    setBusy(true);
    setMessage("");
    try {
      await invoke("write_lcu", {
        operationId: crypto.randomUUID(),
        method,
        path,
        data: data ?? null,
      });
      setMessage(`已完成：${method} ${path}`);
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function removeChallengeTokens() {
    setBusy(true);
    setMessage("");
    try {
      const me = await invoke<{
        lol?: { bannerIdSelected?: string };
      }>("auxiliary_read", { path: "/lol-chat/v1/me" });
      await invoke("write_lcu", {
        operationId: crypto.randomUUID(),
        method: "POST",
        path: "/lol-challenges/v1/update-player-preferences/",
        data: {
          challengeIds: [],
          bannerAccent: me.lol?.bannerIdSelected ?? "",
        },
      });
      setMessage("已移除挑战代币");
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function removePrestigeCrest() {
    setBusy(true);
    setMessage("");
    try {
      const regalia = await invoke<{ preferredBannerType?: string }>(
        "auxiliary_read",
        { path: "/lol-regalia/v2/current-summoner/regalia" },
      );
      await invoke("write_lcu", {
        operationId: crypto.randomUUID(),
        method: "PUT",
        path: "/lol-regalia/v2/current-summoner/regalia",
        data: {
          preferredCrestType: "prestige",
          preferredBannerType: regalia.preferredBannerType ?? "",
          selectedPrestigeCrest: 22,
        },
      });
      setMessage("已移除 prestige 徽章");
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function spectate() {
    setBusy(true);
    setMessage("");
    try {
      await invoke("aux_spectate", {
        operationId: crypto.randomUUID(),
        name: spectateName,
      });
      setMessage("已发起观战");
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function toggleLockConfig(locked: boolean) {
    setBusy(true);
    setMessage("");
    try {
      const result = await invoke<boolean>("set_lock_config", {
        enabled: locked,
      });
      setLockState((current) => ({ ...current, locked: result }));
      setMessage(result ? "已锁定游戏配置" : "已解除配置锁定");
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function fixWindow() {
    setBusy(true);
    setMessage("");
    try {
      await invoke("fix_client_window");
      setMessage("已请求修复客户端窗口（如出现 UAC 请允许）");
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={styles.page}>
      <Title3>辅助工具</Title3>
      {!enabled && (
        <MessageBar intent="warning">
          <MessageBarBody>
            LCU 写操作当前关闭。以环境变量
            {" "}
            <code>SERAPHINE_ENABLE_LCU_WRITES=1</code>
            {" "}
            启动应用后可用；所有请求仍经过写守卫与读回确认。
          </MessageBarBody>
        </MessageBar>
      )}
      {message && (
        <MessageBar intent={message.startsWith("已完成") ? "success" : "error"}>
          <MessageBarBody>{message}</MessageBarBody>
        </MessageBar>
      )}

      <div className={styles.grid}>
        <section className={styles.card}>
          <Title3>在线状态</Title3>
          <Field label="个性签名">
            <Input
              value={statusMessage}
              onChange={(_, data) => setStatusMessage(data.value)}
              placeholder="输入状态文本"
            />
          </Field>
          <Button
            size="small"
            disabled={!enabled || busy}
            onClick={() =>
              void write("/lol-chat/v1/me", "PUT", {
                statusMessage,
              })
            }
          >
            设置状态
          </Button>
          <Field label="可用性">
            <Dropdown
              value={
                AVAILABILITY.find((item) => item.value === availability)?.label ??
                availability
              }
              selectedOptions={[availability]}
              onOptionSelect={(_, data) =>
                setAvailability(data.optionValue ?? "online")
              }
            >
              {AVAILABILITY.map((item) => (
                <Option key={item.value} value={item.value}>
                  {item.label}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Button
            size="small"
            disabled={!enabled || busy}
            onClick={() =>
              void write("/lol-chat/v1/me", "PUT", { availability })
            }
          >
            应用可用性
          </Button>
        </section>

        <section className={styles.card}>
          <Title3>资料展示</Title3>
          <Field label="头像图标 ID">
            <Input
              type="number"
              value={profileIcon}
              onChange={(_, data) => setProfileIcon(data.value)}
              placeholder="例如 4568"
            />
          </Field>
          <Button
            size="small"
            disabled={!enabled || busy || !profileIcon}
            onClick={() =>
              void write("/lol-summoner/v1/current-summoner/icon", "PUT", {
                profileIconId: Number(profileIcon),
              })
            }
          >
            设置头像
          </Button>
          <Field label="背景皮肤 ID">
            <Input
              type="number"
              value={backgroundSkin}
              onChange={(_, data) => setBackgroundSkin(data.value)}
              placeholder="例如 518000"
            />
          </Field>
          <Button
            size="small"
            disabled={!enabled || busy || !backgroundSkin}
            onClick={() =>
              void write(
                "/lol-summoner/v1/current-summoner/summoner-profile",
                "POST",
                {
                  key: "backgroundSkinId",
                  value: Number(backgroundSkin),
                },
              )
            }
          >
            设置背景
          </Button>
        </section>

        <section className={styles.card}>
          <Title3>挑战与徽章</Title3>
          <Button
            size="small"
            disabled={!enabled || busy}
            onClick={() => void removeChallengeTokens()}
          >
            移除挑战代币
          </Button>
          <Button
            size="small"
            disabled={!enabled || busy}
            onClick={() => void removePrestigeCrest()}
          >
            移除 prestige 徽章
          </Button>
          <Caption1 className={styles.muted}>
            会修改账号的展示内容，请确认后再操作。
          </Caption1>
        </section>

        <section className={styles.card}>
          <Title3>资料段位展示</Title3>
          <Field label="队列">
            <Dropdown
              value={
                DISPLAY_QUEUES.find((item) => item.value === displayQueue)?.label ??
                displayQueue
              }
              selectedOptions={[displayQueue]}
              onOptionSelect={(_, data) =>
                setDisplayQueue(data.optionValue ?? "RANKED_SOLO_5x5")
              }
            >
              {DISPLAY_QUEUES.map((item) => (
                <Option key={item.value} value={item.value}>
                  {item.label}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Field label="段位">
            <Dropdown
              value={displayTier}
              selectedOptions={[displayTier]}
              onOptionSelect={(_, data) =>
                setDisplayTier(data.optionValue ?? "GOLD")
              }
            >
              {TIERS.map((tier) => (
                <Option key={tier} value={tier}>
                  {tier}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Field label="小段">
            <Dropdown
              value={displayDivision}
              selectedOptions={[displayDivision]}
              onOptionSelect={(_, data) =>
                setDisplayDivision(data.optionValue ?? "I")
              }
            >
              {DIVISIONS.map((division) => (
                <Option key={division} value={division}>
                  {division}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Button
            size="small"
            disabled={!enabled || busy}
            onClick={() =>
              void write("/lol-chat/v1/me", "PUT", {
                lol: {
                  rankedLeagueQueue: displayQueue,
                  rankedLeagueTier: displayTier,
                  rankedLeagueDivision: displayDivision,
                },
              })
            }
          >
            应用段位展示
          </Button>
        </section>

        <section className={styles.card}>
          <Title3>观战</Title3>
          <Field label="召唤师名称">
            <Input
              value={spectateName}
              onChange={(_, data) => setSpectateName(data.value)}
              placeholder="名称#TAG"
            />
          </Field>
          <Button
            size="small"
            disabled={!enabled || busy || !spectateName}
            onClick={() => void spectate()}
          >
            观战该玩家
          </Button>
          <Caption1 className={styles.muted}>
            玩家需在可观战的对局中；失败时会返回客户端错误。
          </Caption1>
        </section>

        <section className={styles.card}>
          <Title3>游戏配置</Title3>
          <Field label="锁定 PersistedSettings.json（只读）">
            <Switch
              checked={lockState.locked}
              disabled={!lockState.available || busy}
              onChange={(_, data) => void toggleLockConfig(data.checked)}
            />
          </Field>
          {!lockState.available && (
            <Caption1 className={styles.muted}>
              未找到游戏配置文件；请在设置中确认英雄联盟安装目录。
            </Caption1>
          )}
          <Caption1 className={styles.muted}>
            锁定后游戏内设置变更不会写回配置文件；关闭前建议解除锁定。
          </Caption1>
        </section>

        <section className={styles.card}>
          <Title3>客户端</Title3>
          <Button
            size="small"
            disabled={busy}
            onClick={() => void fixWindow()}
          >
            修复客户端窗口
          </Button>
          <Button
            size="small"
            disabled={!enabled || busy}
            onClick={() => void write("/lol-gameflow/v1/reconnect", "POST")}
          >
            强制重连
          </Button>
          <Button
            size="small"
            appearance="outline"
            disabled={!enabled || busy}
            onClick={() => void write("/riotclient/kill-and-restart-ux", "POST")}
          >
            重启客户端
          </Button>
          <Caption1 className={styles.muted}>
            重启会断开当前会话；应用会自动重连。以上操作会立即写入客户端。
          </Caption1>
        </section>
      </div>
    </div>
  );
}