export type Settings = {
  model: string;
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
  };
};

export type SessionSummary = {
  id: string;
  name: string;
  status: string;
  sessionKind: string;
  updatedAt: string;
  messageCount: number;
};

export type TranscriptMessage = {
  id: string;
  role: string;
  content: string;
  entryType: string;
  parentId: string | null;
  timestamp: string;
};

export type PendingApproval = {
  toolCallId: string;
  toolName: string;
  reason: string;
  arguments: unknown;
};

export type BootstrapPayload = {
  settings: Settings;
  shouldRunOnboarding: boolean;
  sessions: SessionSummary[];
  currentSession: SessionSummary;
  transcript: TranscriptMessage[];
  pendingApproval: PendingApproval | null;
};

export type RestorePayload = {
  session: SessionSummary;
  transcript: TranscriptMessage[];
  pendingApproval: PendingApproval | null;
};

export type SubmitPayload = RestorePayload;

export type StreamPayload = {
  turnId: string;
  sessionId: string;
  delta?: string;
  target?: string;
  pendingApproval?: PendingApproval;
  toolCall?: { id: string; name: string; arguments: unknown };
  toolResult?: { name: string; content: string; is_error?: boolean };
};
