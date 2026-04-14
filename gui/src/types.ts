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
};
