import { useEffect, useState } from "react";
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
  Textarea,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import type {
  AppConfig,
  FunctionConfig,
  GeneralConfig,
  PersonalizationConfig,
  ThemeConfig,
} from "./config";

const useStyles = makeStyles({
  page: { display: "flex", flexDirection: "column", gap: "14px" },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "12px",
  },
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
    maxWidth: "368px",
    breakInside: "avoid",
    marginBottom: "12px",
    verticalAlign: "top",
  },
  row: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: "12px",
  },
  group: {
    padding: "6px 8px",
    borderRadius: "6px",
    background: tokens.colorNeutralBackground3,
    display: "flex",
    flexDirection: "column",
    gap: "4px",
  },
  muted: { color: tokens.colorNeutralForeground3 },
});

const DPI_OPTIONS = ["Auto", "1", "1.25", "1.5", "1.75", "2"];
const LANGUAGE_OPTIONS = [
  { value: "Auto", label: "自动" },
  { value: "zh_CN", label: "简体中文" },
  { value: "en_US", label: "English" },
  { value: "pt_BR", label: "Português" },
];
const REGION_OPTIONS = [
  { value: "kr", label: "韩服" },
  { value: "global", label: "全球" },
];
const TIER_OPTIONS = [
  "all",
  "ibsg",
  "gold_plus",
  "platinum_plus",
  "emerald_plus",
  "diamond_plus",
  "master",
  "master_plus",
  "grandmaster",
  "challenger",
];
const POSITION_OPTIONS = ["TOP", "JUNGLE", "MID", "ADC", "SUPPORT"];




const QUEUE_FILTER_ROWS: [string, string][] = [
  ["420", "单双排"],
  ["440", "灵活组排"],
  ["450", "极地大乱斗"],
  ["430", "匹配"],
  ["480", "快速匹配"],
  ["2400", "斗魂竞技场"],
  ["4310", "经典模式"],
  ["2450", "海克斯大乱斗"],
];

export default function SettingsPage({
  config,
  onSaved,
}: {
  config: AppConfig;
  onSaved: (config: AppConfig) => void;
}) {
  const styles = useStyles();
  const [draft, setDraft] = useState<AppConfig>(config);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState("");
  const [queues, setQueues] = useState<Record<string, { name?: string }>>({});
  const [clients, setClients] = useState<
    { pid: number; port?: number; server?: string }[]
  >([]);
  const [currentPid, setCurrentPid] = useState<number | null>(null);
  const [appVersion, setAppVersion] = useState("");
  const [update, setUpdate] = useState<Update | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [updateStatus, setUpdateStatus] = useState("");

  useEffect(() => {
    void invoke<string>("app_version")
      .then(setAppVersion)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    let disposed = false;
    async function load() {
      try {
        const [list, status] = await Promise.all([
          invoke<{ pid: number; port?: number; server?: string }[]>("lcu_clients"),
          invoke<{ connected: boolean; pid: number }>("session_status"),
        ]);
        if (!disposed) {
          setClients(list);
          setCurrentPid(status.connected ? status.pid : null);
        }
      } catch {
        // ignore polling errors
      }
    }
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);

  async function connectClient(pid: number) {
    try {
      await invoke("session_connect", { pid });
      setCurrentPid(pid);
    } catch (cause) {
      setMessage(String(cause));
    }
  }

  async function disconnectClient() {
    try {
      await invoke("session_disconnect");
      setCurrentPid(null);
    } catch (cause) {
      setMessage(String(cause));
    }
  }

  useEffect(() => {
    void invoke<{ queues?: Record<string, { name?: string }> }>("catalog_get")
      .then((catalog) => setQueues(catalog.queues ?? {}))
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    setDraft(config);
  }, [config]);

  useEffect(() => {
    if (!config.general.enableCheckUpdate) return;
    void checkUpdates();
    // 只在页面首次打开时自动检查一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const updateGeneral = <K extends keyof GeneralConfig>(
    key: K,
    value: GeneralConfig[K],
  ) =>
    setDraft((current) => ({
      ...current,
      general: { ...current.general, [key]: value },
    }));

  const updateFunctions = <K extends keyof FunctionConfig>(
    key: K,
    value: FunctionConfig[K],
  ) =>
    setDraft((current) => ({
      ...current,
      functions: { ...current.functions, [key]: value },
    }));

  const updatePersonalization = <K extends keyof PersonalizationConfig>(
    key: K,
    value: PersonalizationConfig[K],
  ) =>
    setDraft((current) => ({
      ...current,
      personalization: { ...current.personalization, [key]: value },
    }));

  const updateTheme = <K extends keyof ThemeConfig>(
    key: K,
    value: ThemeConfig[K],
  ) =>
    setDraft((current) => ({
      ...current,
      theme: { ...current.theme, [key]: value },
    }));

  const queueFilter =
    (draft.functions.queueFilter as Record<string, number[]> | undefined) ?? {};

  function updateQueueFilter(key: string, values: number[]) {
    setDraft((current) => ({
      ...current,
      functions: {
        ...current.functions,
        queueFilter: {
          ...((current.functions.queueFilter as
            | Record<string, number[]>
            | undefined) ?? {}),
          [key]: values,
        },
      },
    }));
  }

  function renderQueueFilterRow(key: string, label: string) {
    const values = queueFilter[key] ?? [];
    const options = Object.entries(queues)
      .map(([id, queue]) => ({ id: Number(id), name: queue.name ?? id }))
      .filter((queue) => !values.includes(queue.id));
    return (
      <div className={styles.group} key={key}>
        <Caption1 className={styles.muted}>{label}</Caption1>
        <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
          {values.map((id) => (
            <Badge key={id} appearance="tint" color="informative">
              {queues[String(id)]?.name ?? `#${id}`}
              <Button
                size="small"
                appearance="transparent"
                icon={<DismissRegular />}
                aria-label="移除"
                onClick={() =>
                  updateQueueFilter(
                    key,
                    values.filter((value) => value !== id),
                  )
                }
              />
            </Badge>
          ))}
          {values.length === 0 && (
            <Caption1 className={styles.muted}>未限制（统计所有队列）</Caption1>
          )}
        </div>
        <Dropdown
          aria-label={`为${label}添加队列`}
          value="添加队列…"
          selectedOptions={[]}
          onOptionSelect={(_, data) => {
            const id = Number(data.optionValue);
            if (id > 0 && !values.includes(id)) {
              updateQueueFilter(key, [...values, id]);
            }
          }}
        >
          {options.map((queue) => (
            <Option
              key={queue.id}
              value={String(queue.id)}
              text={`${queue.name} #${queue.id}`}
            >
              {`${queue.name} #${queue.id}`}
            </Option>
          ))}
        </Dropdown>
      </div>
    );
  }

  async function checkUpdates() {
    setCheckingUpdate(true);
    setUpdateStatus("");
    try {
      const result = await check();
      setUpdate(result);
      setUpdateStatus(
        result
          ? `发现新版本 ${result.version}${result.body ? `：${result.body}` : ""}`
          : "已是最新版本",
      );
    } catch (cause) {
      setUpdate(null);
      setUpdateStatus(`检查失败：${cause}`);
    } finally {
      setCheckingUpdate(false);
    }
  }

  async function installUpdate() {
    if (!update) return;
    setInstallingUpdate(true);
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (cause) {
      setUpdateStatus(`安装失败：${cause}`);
    } finally {
      setInstallingUpdate(false);
    }
  }

  async function save() {
    setSaving(true);
    setMessage("");
    try {
      const updated = await invoke<AppConfig>("config_patch", {
        patch: {
          general: draft.general,
          functions: draft.functions,
          personalization: draft.personalization,
          theme: draft.theme,
        },
      });
      onSaved(updated);
      await invoke("set_mica", {
        enabled: updated.personalization.micaEnabled,
        dark: updated.theme.themeMode.toLowerCase() === "dark",
      }).catch(() => undefined);
      setMessage("设置已保存");
    } catch (cause) {
      setMessage(String(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <Title3>设置</Title3>
        <Button appearance="primary" disabled={saving} onClick={() => void save()}>
          {saving ? "保存中…" : "保存设置"}
        </Button>
      </div>

      {message && (
        <MessageBar intent={message === "设置已保存" ? "success" : "error"}>
          <MessageBarBody>{message}</MessageBarBody>
        </MessageBar>
      )}

      <div className={styles.grid}>
        <section className={styles.card}>
          <Title3>常规</Title3>
          <Field label="启动时检查更新">
            <Switch
              checked={draft.general.enableCheckUpdate}
              onChange={(_, data) =>
                updateGeneral("enableCheckUpdate", data.checked)
              }
            />
          </Field>
          <Field label="关闭窗口时最小化到托盘">
            <Switch
              checked={draft.general.enableCloseToTray}
              onChange={(_, data) =>
                updateGeneral("enableCloseToTray", data.checked)
              }
            />
          </Field>
          <Field label="游戏开始时最小化">
            <Switch
              checked={draft.general.enableGameStartMinimize}
              onChange={(_, data) =>
                updateGeneral("enableGameStartMinimize", data.checked)
              }
            />
          </Field>
          <Field label="随应用启动客户端">
            <Switch
              checked={draft.general.enableStartLolWithApp}
              onChange={(_, data) =>
                updateGeneral("enableStartLolWithApp", data.checked)
              }
            />
          </Field>
          <Field label="静默启动">
            <Switch
              checked={draft.general.enableSilent}
              onChange={(_, data) => updateGeneral("enableSilent", data.checked)}
            />
          </Field>
          <Field label="客户端轮询间隔（毫秒）">
            <Input
              type="number"
              value={String(draft.general.processPollMs)}
              onChange={(_, data) =>
                updateGeneral("processPollMs", Number(data.value) || 1500)
              }
            />
          </Field>
          <Field label="英雄联盟安装目录（每行一个）">
            <Textarea
              value={draft.general.lolPath.join("\n")}
              onChange={(_, data) =>
                updateGeneral(
                  "lolPath",
                  data.value
                    .split(/\r?\n/)
                    .map((line) => line.trim())
                    .filter(Boolean),
                )
              }
            />
          </Field>
        </section>

        <section className={styles.card}>
          <Title3>网络与代理</Title3>
          <Field label="启用 GitHub 代理">
            <Switch
              checked={draft.general.enableGithubProxy}
              onChange={(_, data) =>
                updateGeneral("enableGithubProxy", data.checked)
              }
            />
          </Field>
          <Field label="GitHub 代理地址">
            <Input
              value={draft.general.githubProxyAddr}
              placeholder="127.0.0.1:7890"
              onChange={(_, data) =>
                updateGeneral("githubProxyAddr", data.value)
              }
            />
          </Field>
          <Field label="启用 OP.GG 代理">
            <Switch
              checked={draft.general.enableOpggProxy}
              onChange={(_, data) =>
                updateGeneral("enableOpggProxy", data.checked)
              }
            />
          </Field>
          <Field label="OP.GG 代理地址">
            <Input
              value={draft.general.opggProxyAddr}
              placeholder="127.0.0.1:7890"
              onChange={(_, data) => updateGeneral("opggProxyAddr", data.value)}
            />
          </Field>
          <Caption1 className={styles.muted}>
            代理修改在下次启动时应用到已有连接。
          </Caption1>
        </section>

        <section className={styles.card}>
          <Title3>对局筛选</Title3>
          <Caption1 className={styles.muted}>
            选择每种模式下计入近期战绩的队列；留空表示不限制。
          </Caption1>
          {QUEUE_FILTER_ROWS.map(([key, label]) =>
            renderQueueFilterRow(key, label),
          )}
        </section>

        <section className={styles.card}>
          <Title3>OP.GG 默认值</Title3>
          <Field label="区服">
            <Dropdown
              value={
                REGION_OPTIONS.find(
                  (item) => item.value === draft.functions.opggRegion,
                )?.label ?? draft.functions.opggRegion
              }
              selectedOptions={[draft.functions.opggRegion]}
              onOptionSelect={(_, data) =>
                updateFunctions("opggRegion", data.optionValue ?? "kr")
              }
            >
              {REGION_OPTIONS.map((item) => (
                <Option key={item.value} value={item.value}>
                  {item.label}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Field label="段位">
            <Dropdown
              value={draft.functions.opggTier}
              selectedOptions={[draft.functions.opggTier]}
              onOptionSelect={(_, data) =>
                updateFunctions("opggTier", data.optionValue ?? "emerald_plus")
              }
            >
              {TIER_OPTIONS.map((item) => (
                <Option key={item} value={item}>
                  {item}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Field label="位置">
            <Dropdown
              value={draft.functions.opggPosition}
              selectedOptions={[draft.functions.opggPosition]}
              onOptionSelect={(_, data) =>
                updateFunctions("opggPosition", data.optionValue ?? "ADC")
              }
            >
              {POSITION_OPTIONS.map((item) => (
                <Option key={item} value={item}>
                  {item}
                </Option>
              ))}
            </Dropdown>
          </Field>
        </section>

        <section className={styles.card}>
          <Title3>软件更新</Title3>
          <Caption1 className={styles.muted}>
            当前版本 {appVersion || "2.0.0"}
          </Caption1>
          <div className={styles.row}>
            <Button
              size="small"
              disabled={checkingUpdate || installingUpdate}
              onClick={() => void checkUpdates()}
            >
              {checkingUpdate ? "检查中…" : "检查更新"}
            </Button>
            {update && (
              <Button
                size="small"
                appearance="primary"
                disabled={installingUpdate}
                onClick={() => void installUpdate()}
              >
                {installingUpdate ? "安装中…" : `安装 ${update.version}`}
              </Button>
            )}
          </div>
          {updateStatus && (
            <Caption1 className={styles.muted}>{updateStatus}</Caption1>
          )}
          <Caption1 className={styles.muted}>
            更新源为 GitHub Releases；安装完成后应用会自动重启。
          </Caption1>
        </section>

        <section className={styles.card}>
          <Title3>客户端连接</Title3>
          {clients.length === 0 && (
            <Caption1 className={styles.muted}>未检测到英雄联盟客户端</Caption1>
          )}
          {clients.map((client) => (
            <div className={styles.row} key={client.pid}>
              <div>
                <Caption1>
                  {client.server || "未知区服"} · PID {client.pid} · 端口{" "}
                  {client.port ?? "-"}
                </Caption1>
              </div>
              {currentPid === client.pid ? (
                <Badge appearance="tint" color="success">
                  已连接
                </Badge>
              ) : (
                <Button
                  size="small"
                  onClick={() => void connectClient(client.pid)}
                >
                  连接
                </Button>
              )}
            </div>
          ))}
          {currentPid !== null && (
            <Button
              size="small"
              appearance="subtle"
              onClick={() => void disconnectClient()}
            >
              断开当前连接
            </Button>
          )}
          <Caption1 className={styles.muted}>
            多开时一次只能连接一个客户端；切换后事件流会重新建立。
          </Caption1>
        </section>

        <section className={styles.card}>
          <Title3>外观</Title3>
          <Field label="主题模式">
            <Dropdown
              value={draft.theme.themeMode}
              selectedOptions={[draft.theme.themeMode]}
              onOptionSelect={(_, data) =>
                updateTheme("themeMode", data.optionValue ?? "Light")
              }
            >
              <Option value="Light">浅色</Option>
              <Option value="Dark">深色</Option>
            </Dropdown>
          </Field>
          <Field label="Mica 效果（Windows 11）">
            <Switch
              checked={draft.personalization.micaEnabled}
              onChange={(_, data) =>
                updatePersonalization("micaEnabled", data.checked)
              }
            />
          </Field>
          <Field label="DPI 缩放（重启生效）">
            <Dropdown
              value={draft.personalization.dpiScale}
              selectedOptions={[draft.personalization.dpiScale]}
              onOptionSelect={(_, data) =>
                updatePersonalization("dpiScale", data.optionValue ?? "Auto")
              }
            >
              {DPI_OPTIONS.map((item) => (
                <Option key={item} value={item}>
                  {item}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Field label="语言（重启生效）">
            <Dropdown
              disabled
              value={
                LANGUAGE_OPTIONS.find(
                  (item) => item.value === draft.personalization.language,
                )?.label ?? draft.personalization.language
              }
              selectedOptions={[draft.personalization.language]}
              onOptionSelect={(_, data) =>
                updatePersonalization("language", data.optionValue ?? "Auto")
              }
            >
              {LANGUAGE_OPTIONS.map((item) => (
                <Option key={item.value} value={item.value}>
                  {item.label}
                </Option>
              ))}
            </Dropdown>
          </Field>
          <Caption1 className={styles.muted}>
            当前版本中文优先；英文/葡语切换将在后续提供。
          </Caption1>
          <Field label="窗口尺寸">
            <Input
              value={draft.personalization.windowSize}
              onChange={(_, data) =>
                updatePersonalization("windowSize", data.value)
              }
            />
          </Field>
          <Field label="主题色">
            <Input
              value={draft.theme.themeColor}
              onChange={(_, data) => updateTheme("themeColor", data.value)}
            />
          </Field>
          <Field label="字体（逗号分隔）">
            <Input
              value={draft.theme.fontFamilies.join(", ")}
              onChange={(_, data) =>
                updateTheme(
                  "fontFamilies",
                  data.value
                    .split(",")
                    .map((item) => item.trim())
                    .filter(Boolean),
                )
              }
            />
          </Field>
        </section>
      </div>
    </div>
  );
}