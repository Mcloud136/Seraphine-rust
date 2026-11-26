export interface GeneralConfig {
  [key: string]: unknown;
  backendMode: string;
  enableCheckUpdate: boolean;
  enableCloseToTray: boolean;
  enableGameStartMinimize: boolean;
  enableGithubProxy: boolean;
  enableOpggProxy: boolean;
  enableSilent: boolean;
  enableStartLolWithApp: boolean;
  githubProxyAddr: string;
  logLevel: number;
  lolPath: string[];
  opggProxyAddr: string;
  processPollMs: number;
}

export interface FunctionConfig {
  [key: string]: unknown;
  apiConcurrencyNumber: number;
  aramDataAutoUpdate: boolean;
  aramDataFeedUrl: string;
  autoAcceptCeilSwap: boolean;
  autoAcceptChampTrade: boolean;
  autoAcceptMatchingDelay: number;
  autoBanDelay: number;
  autoBanChampion: number[];
  autoBanChampionBot: number[];
  autoBanChampionJug: number[];
  autoBanChampionMid: number[];
  autoBanChampionSup: number[];
  autoBanChampionTop: number[];
  autoSelectChampion: number[];
  autoSelectChampionBot: number[];
  autoSelectChampionJug: number[];
  autoSelectChampionMid: number[];
  autoSelectChampionSup: number[];
  autoSelectChampionTop: number[];
  autoSetSummonerSpell: number[];
  autoSetSummonerSpellBot: number[];
  autoSetSummonerSpellJug: number[];
  autoSetSummonerSpellMid: number[];
  autoSetSummonerSpellSup: number[];
  autoSetSummonerSpellTop: number[];
  autoShowOpgg: boolean;
  careerGamesNumber: number;
  enableAutoAcceptMatching: boolean;
  enableAutoBanChampion: boolean;
  enableAutoReconnect: boolean;
  enableAutoSelectChampion: boolean;
  enableAutoSelectTimeoutCompleted: boolean;
  enableAutoSetSpells: boolean;
  enableOpggOnTop: boolean;
  enableRandomSkin: boolean;
  enableReserveGameinfo: boolean;
  gameInfoFilter: boolean;
  opggPosition: string;
  opggRegion: string;
  opggTier: string;
  prententBan: boolean;
  queueFilter: Record<string, number[]>;
  showTierInGameInfo: boolean;
}

export interface PersonalizationConfig {
  [key: string]: unknown;
  darkDeathsNumberColor: string;
  dpiScale: string;
  language: string;
  lightDeathsNumberColor: string;
  loseCardColor: string;
  micaEnabled: boolean;
  remakeCardColor: string;
  winCardColor: string;
  windowSize: string;
}

export interface ThemeConfig {
  [key: string]: unknown;
  fontFamilies: string[];
  themeColor: string;
  themeMode: string;
}

export interface AppConfig {
  schemaVersion: number;
  general: GeneralConfig;
  functions: FunctionConfig;
  personalization: PersonalizationConfig;
  theme: ThemeConfig;
  other: { [key: string]: unknown };
}