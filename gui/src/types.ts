export type FileNode = {
  name: string;
  path: string;
  is_dir: boolean;
  children: FileNode[] | null;
};

export type Settings = {
  model: string;
  verbose: boolean;
  onboarding: {
    has_completed_onboarding: boolean;
    last_onboarding_version: string;
  };
  api: {
    provider: string;
    protocol: string;
    api_key: string | null;
    base_url: string;
    streaming: boolean;
    max_tokens: number;
    timeout: number;
  };
  memory: {
    enabled: boolean;
    path: string;
    consolidation_interval: number;
    max_memories: number;
  };
  voice: {
    enabled: boolean;
    push_to_talk: boolean;
    silence_threshold: number;
    sample_rate: number;
  };
  compact: {
    enabled: boolean;
    auto_compact: boolean;
    max_tokens_before_compact: number;
  };
  permissions: {
    mode: 'allow_all' | 'ask' | 'deny_all';
  };
  session: {
    auto_restore_last_session: boolean;
    persist_transcript: boolean;
  };
};

export type SessionSummary = {
  id: string;
  name: string;
  status: string;
  sessionKind: string;
  updatedAt: string;
  message_count: number; // 对齐后端 DTO
};

export type TranscriptMessage = {
  id: string;
  role: string;
  content: string;
  entry_type: string;
  parent_id: string | null;
  timestamp: string;
};

export type BootstrapPayload = {
  projectName: string;
  projectPath: string;
  settings: Settings;
  should_run_onboarding: boolean;
  sessions: SessionSummary[];
  currentSession: SessionSummary;
  transcript: TranscriptMessage[];
  pending_approval: any | null;
};

export type SubmitPayload = {
  session: SessionSummary;
  transcript: TranscriptMessage[];
  pending_approval: any | null;
};

export type RestorePayload = SubmitPayload;

export type StreamPayload = {
  turnId: string;
  sessionId: string;
  delta?: string;
  target?: string;
  error?: string;
  toolCall?: ToolCall;
  toolResult?: ToolResult;
  pendingApproval?: any;
};

export type ToolCall = {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
};

export type ToolResult = {
  tool_call_id: string;
  name: string;
  is_error: boolean;
  content?: string;
};

export type RuntimeUsage = {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
};

export type McpServerInfo = {
  name: string;
  status: string;
  tools_count: number;
  resources_count: number;
  prompts_count: number;
  started_at: string | null;
  last_error: string | null;
};

export type McpConfig = {
  name: string;
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string | null;
  status: string;
  capabilities: string[];
  auto_start: boolean;
};

export type TurnCompletedPayload = {
  turnId: string;
  sessionId: string;
  transcript: TranscriptMessage[];
  pendingApproval: any | null;
  usage: RuntimeUsage | null;
  toolCallCount: number;
};
