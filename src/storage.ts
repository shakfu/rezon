import {
  Conversation,
  DEFAULT_SETTINGS,
  Settings,
  Theme,
} from "./types";

const KEY_CONVS = "rezo:conversations";
const KEY_CURRENT = "rezo:currentConversationId";
const KEY_SETTINGS = "rezo:settings";

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

export function deriveTitle(firstUserMessage: string): string {
  const trimmed = firstUserMessage.trim().replace(/\s+/g, " ");
  if (!trimmed) return "New chat";
  return trimmed.length > 60 ? trimmed.slice(0, 57) + "..." : trimmed;
}
