export type Role = "system" | "user" | "assistant" | "tool";

export type MsgStats = {
  provider: string;
  promptTokens?: number;
  cachedTokens?: number;
  genTokens: number;
  durationMs: number;
};

/// One tool invocation from the agent. Stored on the assistant `Msg`
/// it belongs to and rendered as a pill (collapsed) or as args/result
/// (expanded). `status` advances running -> ok | error.
export type ToolCallEntry = {
  id: string;
  name: string;
  arguments?: string;
  status: "running" | "ok" | "error";
  result?: unknown;
  error?: string;
};

export type Msg = {
  role: Role;
  content: string;
  isError?: boolean;
  stats?: MsgStats;
  /// Populated on assistant messages that requested tool calls.
  toolCalls?: ToolCallEntry[];
  /// Populated on tool messages (the result threaded back to the
  /// model on the next turn). Hidden from the chat bubble; rendered
  /// only when the user expands the matching pill on the prior
  /// assistant turn.
  toolCallId?: string;
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

/// Per-tool permission. "ask" prompts on every call; "always" skips
/// confirmation and dispatches; "disable" filters the tool out of the
/// catalog the model sees, so it cannot be invoked at all. Until the
/// confirmation flow lands (phase 5), "ask" behaves like "always" with
/// a stderr warning.
export type ToolPermission = "ask" | "always" | "disable";

export type ToolPermissions = Record<string, ToolPermission>;

export type ToolInfo = {
  name: string;
  description: string;
  requiresConfirmation: boolean;
};

export type Settings = {
  theme: Theme;
  fontSize: number;
  defaultSystemPrompt: string;
  leftSidebarCollapsed: boolean;
  rightSidebarCollapsed: boolean;
  contextOverflow: ContextOverflow;
  /// When true, send turns through the tool-aware agent loop instead
  /// of the plain chat command. Both cloud and local supported.
  toolsEnabled: boolean;
  /// Per-tool permission. Tools missing from the map default to "ask".
  toolPermissions: ToolPermissions;
};

export const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  fontSize: 14,
  defaultSystemPrompt: "You are a concise, helpful assistant.",
  leftSidebarCollapsed: false,
  rightSidebarCollapsed: false,
  contextOverflow: "error",
  toolsEnabled: false,
  toolPermissions: {},
};

export function toolPermissionFor(
  permissions: ToolPermissions,
  tool: ToolInfo,
): ToolPermission {
  return (
    permissions[tool.name] ?? (tool.requiresConfirmation ? "ask" : "always")
  );
}
