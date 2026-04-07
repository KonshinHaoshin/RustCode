import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type {
  BootstrapPayload,
  PendingApproval,
  RestorePayload,
  SessionSummary,
  Settings,
  StreamPayload,
  SubmitPayload,
  TranscriptMessage,
} from './types';

type ActivityItem = { kind: string; text: string };

const EMPTY_SETTINGS: Settings = {
  model: '',
  onboarding: {
    has_completed_onboarding: false,
    last_onboarding_version: '',
  },
  api: {
    provider: 'deepseek',
    protocol: 'open_ai',
    api_key: null,
    base_url: '',
    streaming: true,
  },
};

export default function App() {
  const [settings, setSettings] = useState<Settings>(EMPTY_SETTINGS);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [currentSession, setCurrentSession] = useState<SessionSummary | null>(null);
  const [transcript, setTranscript] = useState<TranscriptMessage[]>([]);
  const [composer, setComposer] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [thinking, setThinking] = useState('');
  const [draftAssistant, setDraftAssistant] = useState('');
  const [activity, setActivity] = useState<ActivityItem[]>([]);
  const [pendingApproval, setPendingApproval] = useState<PendingApproval | null>(null);

  useEffect(() => {
    const unlisteners: Promise<UnlistenFn>[] = [
      listen<StreamPayload>('thinking_text_chunk', (event) => {
        setThinking((current) => current + (event.payload.delta ?? ''));
      }),
      listen<StreamPayload>('assistant_text_chunk', (event) => {
        setDraftAssistant((current) => current + (event.payload.delta ?? ''));
      }),
      listen<StreamPayload>('tool_call', (event) => {
        const tool = event.payload.toolCall;
        if (!tool) return;
        setActivity((current) => [{ kind: 'tool', text: `Calling ${tool.name}` }, ...current].slice(0, 10));
      }),
      listen<StreamPayload>('tool_result', (event) => {
        const result = event.payload.toolResult;
        if (!result) return;
        setActivity((current) => [{ kind: 'result', text: `${result.name}: ${String(result.content).slice(0, 120)}` }, ...current].slice(0, 10));
      }),
      listen<StreamPayload>('awaiting_approval', (event) => {
        setPendingApproval(event.payload.pendingApproval ?? null);
      }),
      listen<StreamPayload>('turn_completed', () => {
        setThinking('');
        setDraftAssistant('');
        setSending(false);
      }),
    ];

    void bootstrap();

    return () => {
      void Promise.all(unlisteners).then((items) => items.forEach((off) => off()));
    };
  }, []);

  const visibleTranscript = useMemo(() => {
    if (!draftAssistant) {
      return transcript;
    }
    return [
      ...transcript,
      {
        id: 'draft-assistant',
        role: 'assistant',
        content: draftAssistant,
        entryType: 'message',
        parentId: null,
        timestamp: new Date().toISOString(),
      },
    ];
  }, [draftAssistant, transcript]);

  async function bootstrap() {
    setLoading(true);
    setError(null);
    try {
      const payload = await invoke<BootstrapPayload>('bootstrap_gui_state');
      applyBootstrap(payload);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }

  function applyBootstrap(payload: BootstrapPayload) {
    setSettings(payload.settings);
    setSessions(payload.sessions);
    setCurrentSession(payload.currentSession);
    setTranscript(payload.transcript);
    setPendingApproval(payload.pendingApproval);
    setShowOnboarding(payload.shouldRunOnboarding);
  }

  async function saveCurrentSettings(markOnboardingDone = false) {
    setSaving(true);
    setError(null);
    try {
      const saved = await invoke<Settings>('save_settings', { settings });
      setSettings(saved);
      if (markOnboardingDone) {
        const completed = await invoke<Settings>('complete_onboarding');
        setSettings(completed);
        setShowOnboarding(false);
      }
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  }

  async function restoreSession(sessionId: string) {
    setError(null);
    try {
      const payload = await invoke<RestorePayload>('restore_session', { sessionId });
      setCurrentSession(payload.session);
      setTranscript(payload.transcript);
      setPendingApproval(payload.pendingApproval);
      setThinking('');
      setDraftAssistant('');
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function submitPrompt() {
    const prompt = composer.trim();
    if (!prompt || sending) {
      return;
    }

    setComposer('');
    setSending(true);
    setThinking('');
    setDraftAssistant('');
    setPendingApproval(null);
    setTranscript((current) => [
      ...current,
      {
        id: `local-${Date.now()}`,
        role: 'user',
        content: prompt,
        entryType: 'message',
        parentId: null,
        timestamp: new Date().toISOString(),
      },
    ]);

    try {
      const payload = await invoke<SubmitPayload>('submit_prompt', { prompt });
      commitTurn(payload);
    } catch (cause) {
      setError(String(cause));
      setSending(false);
    }
  }

  function commitTurn(payload: SubmitPayload) {
    setCurrentSession(payload.session);
    setTranscript(payload.transcript);
    setPendingApproval(payload.pendingApproval);
    setThinking('');
    setDraftAssistant('');
    setSending(false);
    void refreshSessions();
  }

  async function refreshSessions() {
    const next = await invoke<SessionSummary[]>('list_sessions');
    setSessions(next);
  }

  async function handleApproval(action: 'allow_once' | 'deny_once' | 'always_allow' | 'always_deny') {
    setSending(true);
    setThinking('');
    setDraftAssistant('');
    try {
      const payload = await invoke<SubmitPayload>('respond_to_approval', { action });
      commitTurn(payload);
    } catch (cause) {
      setError(String(cause));
      setSending(false);
    }
  }

  if (loading) {
    return <div className="screen center">Loading desktop shell…</div>;
  }

  return (
    <div className="screen shell">
      <aside className="sidebar">
        <div>
          <div className="eyebrow">RustCode Desktop</div>
          <h1>Transcript-first desktop shell</h1>
          <p className="muted">Current session: {currentSession?.name ?? 'none'}</p>
        </div>

        <section className="panel">
          <div className="panelHeader">
            <h2>Sessions</h2>
            <button className="ghostButton" onClick={() => void refreshSessions()} type="button">
              Refresh
            </button>
          </div>
          <div className="sessionList">
            {sessions.map((session) => (
              <button
                key={session.id}
                className={`sessionItem ${session.id === currentSession?.id ? 'active' : ''}`}
                onClick={() => void restoreSession(session.id)}
                type="button"
              >
                <span>{session.name}</span>
                <small>
                  {session.status} · {session.messageCount} msgs
                </small>
              </button>
            ))}
          </div>
        </section>

        <section className="panel">
          <h2>Settings</h2>
          <label>
            Provider
            <input
              value={settings.api.provider}
              onChange={(event) => setSettings((current) => ({ ...current, api: { ...current.api, provider: event.target.value } }))}
            />
          </label>
          <label>
            Protocol
            <input
              value={settings.api.protocol}
              onChange={(event) => setSettings((current) => ({ ...current, api: { ...current.api, protocol: event.target.value } }))}
            />
          </label>
          <label>
            Base URL
            <input
              value={settings.api.base_url}
              onChange={(event) => setSettings((current) => ({ ...current, api: { ...current.api, base_url: event.target.value } }))}
            />
          </label>
          <label>
            API Key
            <input
              type="password"
              value={settings.api.api_key ?? ''}
              onChange={(event) => setSettings((current) => ({ ...current, api: { ...current.api, api_key: event.target.value || null } }))}
            />
          </label>
          <label>
            Model
            <input
              value={settings.model}
              onChange={(event) => setSettings((current) => ({ ...current, model: event.target.value }))}
            />
          </label>
          <button className="primaryButton" disabled={saving} onClick={() => void saveCurrentSettings()} type="button">
            {saving ? 'Saving…' : 'Save settings'}
          </button>
        </section>

        <section className="panel activityPanel">
          <h2>Runtime activity</h2>
          {thinking ? <pre className="thinking">{thinking}</pre> : <p className="muted">No active thinking stream.</p>}
          {activity.map((item, index) => (
            <div className="activityItem" key={`${item.kind}-${index}`}>
              <span>{item.kind}</span>
              <p>{item.text}</p>
            </div>
          ))}
        </section>
      </aside>

      <main className="mainPane">
        <div className="transcript">
          {visibleTranscript.map((message) => (
            <article key={message.id} className={`bubble ${message.role}`}>
              <header>
                <strong>{message.role}</strong>
                <time>{new Date(message.timestamp).toLocaleTimeString()}</time>
              </header>
              <pre>{message.content || '(empty)'}</pre>
            </article>
          ))}
        </div>

        {pendingApproval ? (
          <section className="approvalCard">
            <div>
              <div className="eyebrow">Approval required</div>
              <h3>{pendingApproval.toolName}</h3>
              <p>{pendingApproval.reason}</p>
              <pre>{JSON.stringify(pendingApproval.arguments, null, 2)}</pre>
            </div>
            <div className="approvalActions">
              <button onClick={() => void handleApproval('allow_once')} type="button">Allow once</button>
              <button onClick={() => void handleApproval('always_allow')} type="button">Always allow</button>
              <button onClick={() => void handleApproval('deny_once')} type="button">Deny once</button>
              <button onClick={() => void handleApproval('always_deny')} type="button">Always deny</button>
            </div>
          </section>
        ) : null}

        <footer className="composer">
          <textarea
            placeholder="Ask RustCode to inspect, edit, or explain the project…"
            value={composer}
            onChange={(event) => setComposer(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey) {
                event.preventDefault();
                void submitPrompt();
              }
            }}
          />
          <button className="primaryButton" disabled={sending} onClick={() => void submitPrompt()} type="button">
            {sending ? 'Running…' : 'Send'}
          </button>
        </footer>
      </main>

      {showOnboarding ? (
        <div className="overlay">
          <div className="overlayCard">
            <div className="eyebrow">First run</div>
            <h2>Finish desktop onboarding</h2>
            <p>
              The desktop shell reuses the existing Rust runtime. Confirm your provider, base URL,
              API key, and model, then continue.
            </p>
            <button className="primaryButton" disabled={saving} onClick={() => void saveCurrentSettings(true)} type="button">
              {saving ? 'Saving…' : 'Complete onboarding'}
            </button>
          </div>
        </div>
      ) : null}

      {error ? <div className="errorBanner">{error}</div> : null}
    </div>
  );
}
