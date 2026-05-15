import {
  Conversation,
  DEFAULT_SETTINGS,
  Settings,
  Theme,
} from "./types";

const KEY_CONVS = "rezon:conversations";
const KEY_CURRENT = "rezon:currentConversationId";
const KEY_SETTINGS = "rezon:settings";
const KEY_LAST_PROVIDER = "rezon:lastProvider";
const KEY_CLOUD_MODELS = "rezon:cloudModels";
const KEY_CLOUD_BASE_URLS = "rezon:cloudBaseUrls";

export function loadConversations(): Conversation[] {
  try {
    const raw = localStorage.getItem(KEY_CONVS);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (c: unknown): c is Conversation =>
        !!c &&
        typeof c === "object" &&
        typeof (c as Conversation).id === "string" &&
        Array.isArray((c as Conversation).messages),
    );
  } catch {
    return [];
  }
}

export function saveConversations(convs: Conversation[]) {
  try {
    localStorage.setItem(KEY_CONVS, JSON.stringify(convs));
  } catch {
    /* quota or serialization — ignore */
  }
}

export function loadCurrentId(): string | null {
  return localStorage.getItem(KEY_CURRENT);
}

export function saveCurrentId(id: string | null) {
  if (id) localStorage.setItem(KEY_CURRENT, id);
  else localStorage.removeItem(KEY_CURRENT);
}

export function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(KEY_SETTINGS);
    if (!raw) return DEFAULT_SETTINGS;
    const parsed = JSON.parse(raw) as Partial<Settings>;
    return {
      theme: validateTheme(parsed.theme),
      fontSize:
        typeof parsed.fontSize === "number" &&
        parsed.fontSize >= 10 &&
        parsed.fontSize <= 24
          ? parsed.fontSize
          : DEFAULT_SETTINGS.fontSize,
      defaultSystemPrompt:
        typeof parsed.defaultSystemPrompt === "string"
          ? parsed.defaultSystemPrompt
          : DEFAULT_SETTINGS.defaultSystemPrompt,
      leftSidebarCollapsed:
        typeof parsed.leftSidebarCollapsed === "boolean"
          ? parsed.leftSidebarCollapsed
          : DEFAULT_SETTINGS.leftSidebarCollapsed,
      rightSidebarCollapsed:
        typeof parsed.rightSidebarCollapsed === "boolean"
          ? parsed.rightSidebarCollapsed
          : DEFAULT_SETTINGS.rightSidebarCollapsed,
      contextOverflow:
        parsed.contextOverflow === "slide" || parsed.contextOverflow === "error"
          ? parsed.contextOverflow
          : DEFAULT_SETTINGS.contextOverflow,
      toolsEnabled:
        typeof parsed.toolsEnabled === "boolean"
          ? parsed.toolsEnabled
          : DEFAULT_SETTINGS.toolsEnabled,
      toolPermissions: validateToolPermissions(parsed.toolPermissions),
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

function validateTheme(t: unknown): Theme {
  return t === "light" || t === "dark" || t === "system"
    ? t
    : DEFAULT_SETTINGS.theme;
}

function validateToolPermissions(
  raw: unknown,
): import("./types").ToolPermissions {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  const out: Record<string, "ask" | "always" | "disable"> = {};
  for (const [k, v] of Object.entries(raw)) {
    if (v === "ask" || v === "always" || v === "disable") out[k] = v;
  }
  return out;
}

export function saveSettings(s: Settings) {
  try {
    localStorage.setItem(KEY_SETTINGS, JSON.stringify(s));
  } catch {
    /* ignore */
  }
}

export function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

export function newConversation(systemPrompt: string): Conversation {
  const now = Date.now();
  return {
    id: newId(),
    title: "New chat",
    systemPrompt,
    messages: [],
    createdAt: now,
    updatedAt: now,
  };
}

// ---- Provider + per-provider model persistence ------------------------
//
// We persist the last selected provider and the user's per-provider model
// override (and base URL for the user-configurable "other" provider) so a
// fresh launch reproduces the previous session's model choice.
//
// API keys are deliberately NOT persisted: named providers read theirs
// from env, and persisting the "other" key in localStorage would put a
// secret in clear text. Re-entry on launch is the price.

export function loadLastProvider(): string | null {
  return localStorage.getItem(KEY_LAST_PROVIDER);
}

export function saveLastProvider(provider: string) {
  try {
    localStorage.setItem(KEY_LAST_PROVIDER, provider);
  } catch {
    /* ignore */
  }
}

function loadStringMap(key: string): Record<string, string> {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (typeof v === "string") out[k] = v;
    }
    return out;
  } catch {
    return {};
  }
}

function saveStringMap(key: string, map: Record<string, string>) {
  try {
    localStorage.setItem(key, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

export function loadCloudModels(): Record<string, string> {
  return loadStringMap(KEY_CLOUD_MODELS);
}

export function saveCloudModels(map: Record<string, string>) {
  saveStringMap(KEY_CLOUD_MODELS, map);
}

export function loadCloudBaseUrls(): Record<string, string> {
  return loadStringMap(KEY_CLOUD_BASE_URLS);
}

export function saveCloudBaseUrls(map: Record<string, string>) {
  saveStringMap(KEY_CLOUD_BASE_URLS, map);
}

export function deriveTitle(firstUserMessage: string): string {
  const trimmed = firstUserMessage.trim().replace(/\s+/g, " ");
  if (!trimmed) return "New chat";
  return trimmed.length > 60 ? trimmed.slice(0, 57) + "..." : trimmed;
}
