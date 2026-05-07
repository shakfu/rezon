export type Role = "system" | "user" | "assistant";

export type MsgStats = {
  provider: string;
  promptTokens?: number;
  cachedTokens?: number;
  genTokens: number;
  durationMs: number;
};

export type Msg = {
  role: Role;
  content: string;
  isError?: boolean;
  stats?: MsgStats;
};

export type ModelStatus = { loaded: boolean; path: string | null };

export type CloudProviderInfo = {
  key: string;
  label: string;
  envVar: string;
  defaultModel: string;
  recommendedModels: string[];
  apiKeySet: boolean;
  userConfigurable: boolean;
};

export type Conversation = {
  id: string;
  title: string;
  systemPrompt: string;
  messages: Msg[];
  createdAt: number;
  updatedAt: number;
};

export type Theme = "system" | "light" | "dark";

export type ContextOverflow = "error" | "slide";

export type Settings = {
  theme: Theme;
  fontSize: number;
  defaultSystemPrompt: string;
  leftSidebarCollapsed: boolean;
  rightSidebarCollapsed: boolean;
  contextOverflow: ContextOverflow;
};

export const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  fontSize: 14,
  defaultSystemPrompt: "You are a concise, helpful assistant.",
  leftSidebarCollapsed: false,
  rightSidebarCollapsed: false,
  contextOverflow: "error",
};
