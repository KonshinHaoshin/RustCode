import { useEffect, useMemo, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  BootstrapPayload,
  PendingApproval,
  RestorePayload,
  RewindPreview,
  SessionSummary,
  Settings,
  StreamPayload,
  SubmitPayload,
  TaskSummary,
  TranscriptMessage,
  TurnTarget,
} from './types';

type ActivityItem = {
  id: string;
  kind: string;
  text: string;
};

type AppView = 'home' | 'thread' | 'settings' | 'automation';

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

const SUGGESTIONS = [
  'Review how this project should be installed',
  'Summarize the current Android app structure',
  'Draft the next implementation phase for this repo',
];

function summarize(text: string, max = 80) {
  const compact = text.replace(/\s+/g, ' ').trim();
  if (!compact) return '(empty)';
  return compact.length > max ? `${compact.slice(0, max)}...` : compact;
}

function relativeTime(timestamp: string) {
  const millis = new Date(timestamp).getTime();
  if (Number.isNaN(millis)) return '--';
  const delta = Math.max(1, Math.floor((Date.now() - millis) / 1000));
  if (delta < 60) return `${delta}s`;
  const minutes = Math.floor(delta / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function roleLabel(role: string) {
  if (role === 'user') return 'You';
  if (role === 'assistant') return 'RustCode';
  return 'System';
}

function sessionDetail(session: SessionSummary) {
  return `${session.sessionKind} · ${session.messageCount} messages`;
}

function MarkdownBlock({ content }: { content: string }) {
  const lines = content.split('\n');
  const blocks: ReactNode[] = [];
  let inCode = false;
  let codeLines: string[] = [];

  const flushCode = (key: string) => {
    if (codeLines.length === 0) return;
    blocks.push(
      <pre className="threadCode" key={key}>
        <code>{codeLines.join('\n')}</code>
      </pre>,
    );
    codeLines = [];
  };

  lines.forEach((raw, index) => {
    const line = raw.replace(/\r/g, '');
    if (line.trim().startsWith('```')) {
      if (inCode) flushCode(`code-${index}`);
      inCode = !inCode;
      return;
    }

    if (inCode) {
      codeLines.push(line);
      return;
    }

    if (!line.trim()) {
      blocks.push(<div className="threadGap" key={`gap-${index}`} />);
      return;
    }

    if (line.startsWith('# ')) {
      blocks.push(
        <h1 className="threadH1" key={`h1-${index}`}>
          {line.slice(2)}
        </h1>,
      );
      return;
    }

    if (line.startsWith('## ')) {
      blocks.push(
        <h2 className="threadH2" key={`h2-${index}`}>
          {line.slice(3)}
        </h2>,
      );
      return;
    }

    if (line.startsWith('### ')) {
      blocks.push(
        <h3 className="threadH3" key={`h3-${index}`}>
          {line.slice(4)}
        </h3>,
      );
      return;
    }

    if (/^\s*[-*]\s+/.test(line)) {
      blocks.push(
        <div className="threadListRow" key={`li-${index}`}>
          <span className="threadListBullet">•</span>
          <span>{line.replace(/^\s*[-*]\s+/, '')}</span>
        </div>,
      );
      return;
    }

    if (line.startsWith('> ')) {
      blocks.push(
        <blockquote className="threadQuote" key={`quote-${index}`}>
          {line.slice(2)}
        </blockquote>,
      );
      return;
    }

    const chunks = line.split(/(`[^`]+`)/g);
    blocks.push(
      <p className="threadParagraph" key={`p-${index}`}>
        {chunks.map((chunk, chunkIndex) =>
          chunk.startsWith('`') && chunk.endsWith('`') ? (
            <code key={chunkIndex}>{chunk.slice(1, -1)}</code>
          ) : (
            <span key={chunkIndex}>{chunk}</span>
          ),
        )}
      </p>,
    );
  });

  if (inCode) flushCode('code-tail');

  return <div className="threadMarkdown">{blocks}</div>;
}

export default function App() {
  const [view, setView] = useState<AppView>('home');
  const [projectName, setProjectName] = useState('rustcode');
  const [projectPath, setProjectPath] = useState('');
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
  const [targets, setTargets] = useState<TurnTarget[]>([]);
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [selectedTarget, setSelectedTarget] = useState('');
  const [rewindPreview, setRewindPreview] = useState<RewindPreview | null>(null);
  const [turnFailure, setTurnFailure] = useState<string | null>(null);

  useEffect(() => {
    const listeners: Promise<UnlistenFn>[] = [
      listen<StreamPayload>('thinking_text_chunk', (event) => {
        setThinking((current) => current + (event.payload.delta ?? ''));
      }),
      listen<StreamPayload>('assistant_text_chunk', (event) => {
        setDraftAssistant((current) => current + (event.payload.delta ?? ''));
      }),
      listen<StreamPayload>('model_request', (event) => {
        pushActivity('model', event.payload.target ?? 'Model request sent');
      }),
      listen<StreamPayload>('tool_call', (event) => {
        const tool = event.payload.toolCall;
        if (!tool) return;
        pushActivity('tool', `Calling ${tool.name}`);
      }),
      listen<StreamPayload>('tool_result', (event) => {
        const result = event.payload.toolResult;
        if (!result) return;
        pushActivity('result', `${result.name}: ${summarize(String(result.content), 72)}`);
      }),
      listen<StreamPayload>('awaiting_approval', (event) => {
        setPendingApproval(event.payload.pendingApproval ?? null);
        setSending(false);
      }),
      listen<StreamPayload>('turn_failed', (event) => {
        setTurnFailure(event.payload.error ?? 'Turn failed');
        setThinking('');
        setSending(false);
      }),
      listen<StreamPayload>('turn_completed', async () => {
        setThinking('');
        setDraftAssistant('');
        setTurnFailure(null);
        setSending(false);
        await refreshSideData();
      }),
    ];

    void bootstrap();

    return () => {
      void Promise.all(listeners).then((items) => items.forEach((dispose) => dispose()));
    };
  }, []);

  const visibleTranscript = useMemo(() => {
    const items = [...transcript];
    if (draftAssistant) {
      items.push({
        id: 'draft-assistant',
        role: 'assistant',
        content: draftAssistant,
        entryType: 'message',
        parentId: null,
        timestamp: new Date().toISOString(),
      });
    }
    if (turnFailure) {
      items.push({
        id: 'turn-failure',
        role: 'system',
        content: `Turn failed: ${turnFailure}`,
        entryType: 'system_notice',
        parentId: null,
        timestamp: new Date().toISOString(),
      });
    }
    return items;
  }, [draftAssistant, transcript, turnFailure]);

  function pushActivity(kind: string, text: string) {
    setActivity((current) => [{ id: crypto.randomUUID(), kind, text }, ...current].slice(0, 8));
  }

  function applyBootstrap(payload: BootstrapPayload) {
    setProjectName(payload.projectName);
    setProjectPath(payload.projectPath);
    setSettings(payload.settings);
    setSessions(payload.sessions);
    setCurrentSession(payload.currentSession);
    setTranscript(payload.transcript);
    setPendingApproval(payload.pendingApproval);
    setShowOnboarding(payload.shouldRunOnboarding);
    setSelectedTarget('');
    setRewindPreview(null);
    setDraftAssistant('');
    setThinking('');
    setTurnFailure(null);
    setView(payload.transcript.length > 0 ? 'thread' : 'home');
  }

  async function bootstrap() {
    setLoading(true);
    setError(null);
    try {
      const payload = await invoke<BootstrapPayload>('bootstrap_gui_state');
      applyBootstrap(payload);
      await refreshSideData();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }

  async function refreshSideData() {
    try {
      const [nextSessions, nextTargets, nextTasks] = await Promise.all([
        invoke<SessionSummary[]>('list_sessions'),
        invoke<TurnTarget[]>('list_user_turn_targets'),
        invoke<TaskSummary[]>('list_active_tasks'),
      ]);
      setSessions(nextSessions);
      setTargets(nextTargets);
      setTasks(nextTasks);
      if (!selectedTarget && nextTargets.length > 0) {
        setSelectedTarget(nextTargets[0].messageId);
      }
    } catch (cause) {
      setError(String(cause));
    }
  }

  function applyRestore(payload: RestorePayload | SubmitPayload) {
    setCurrentSession(payload.session);
    setTranscript(payload.transcript);
    setPendingApproval(payload.pendingApproval);
    setDraftAssistant('');
    setThinking('');
    setRewindPreview(null);
    setTurnFailure(null);
    setView(payload.transcript.length > 0 ? 'thread' : 'home');
    void refreshSideData();
  }

  async function restoreSession(sessionId: string) {
    try {
      const payload = await invoke<RestorePayload>('restore_session', { sessionId });
      applyRestore(payload);
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function createSession() {
    try {
      const payload = await invoke<RestorePayload>('create_session');
      setComposer('');
      applyRestore(payload);
      setView('home');
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function deleteSession(sessionId: string) {
    if (!window.confirm('Delete this session? This cannot be undone.')) {
      return;
    }
    try {
      const payload = await invoke<RestorePayload>('delete_session', { sessionId });
      applyRestore(payload);
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function openProjectFolder() {
    try {
      await invoke('open_project_folder');
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function chooseWorkingDirectory() {
    try {
      const payload = await invoke<BootstrapPayload | null>('choose_working_directory');
      if (!payload) return;
      applyBootstrap(payload);
      await refreshSideData();
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function saveCurrentSettings(finishOnboarding = false) {
    setSaving(true);
    try {
      const saved = await invoke<Settings>('save_settings', { next: settings });
      setSettings(saved);
      if (finishOnboarding) {
        await invoke<Settings>('complete_onboarding');
        setShowOnboarding(false);
      }
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  }

  async function runPrompt() {
    const prompt = composer.trim();
    if (!prompt || sending) return;
    setSending(true);
    setError(null);
    setComposer('');
    setView('thread');
    setDraftAssistant('');
    setThinking('');
    setTurnFailure(null);
    setTranscript((current) => [
      ...current,
      {
        id: crypto.randomUUID(),
        role: 'user',
        content: prompt,
        entryType: 'message',
        parentId: null,
        timestamp: new Date().toISOString(),
      },
    ]);
    try {
      const payload = await invoke<SubmitPayload>('submit_prompt', { prompt });
      applyRestore(payload);
    } catch (cause) {
      setError(String(cause));
      setSending(false);
    }
  }

  async function handleApproval(action: 'allow_once' | 'deny_once' | 'always_allow' | 'always_deny') {
    setSending(true);
    try {
      const payload = await invoke<SubmitPayload>('respond_to_approval', { action });
      applyRestore(payload);
    } catch (cause) {
      setError(String(cause));
      setSending(false);
    }
  }

  async function loadPreview(messageId: string) {
    try {
      const preview = await invoke<RewindPreview>('preview_rewind', { messageId });
      setSelectedTarget(messageId);
      setRewindPreview(preview);
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function rewind(filesOnly: boolean) {
    if (!selectedTarget) return;
    try {
      const payload = await invoke<RestorePayload>('rewind_session', {
        messageId: selectedTarget,
        filesOnly,
      });
      applyRestore(payload);
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function createBranch() {
    try {
      const payload = await invoke<RestorePayload>('branch_session', {
        messageId: selectedTarget || null,
      });
      applyRestore(payload);
    } catch (cause) {
      setError(String(cause));
    }
  }

  function onComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      void runPrompt();
    }
  }

  function renderThreadView() {
    if (!currentSession) return null;

    return (
      <section className="threadView">
        <header className="threadHeader">
          <div>
            <div className="eyebrow">Session</div>
            <h2>{currentSession.name}</h2>
          </div>
          <div className="threadHeaderActions">
            <button className="softButton" onClick={() => void createBranch()} type="button">
              Branch
            </button>
            <button
              className="softButton"
              disabled={!selectedTarget}
              onClick={() => selectedTarget && void loadPreview(selectedTarget)}
              type="button"
            >
              Preview restore
            </button>
            <button className="softButton" disabled={!selectedTarget} onClick={() => void rewind(false)} type="button">
              Restore chat
            </button>
          </div>
        </header>

        {pendingApproval ? (
          <div className="inlineNotice warning">
            <div>
              <strong>{pendingApproval.toolName}</strong>
              <p>{pendingApproval.reason}</p>
            </div>
            <div className="noticeActions">
              <button className="softButton" onClick={() => void handleApproval('allow_once')} type="button">
                Allow once
              </button>
              <button className="softButton" onClick={() => void handleApproval('deny_once')} type="button">
                Deny once
              </button>
            </div>
          </div>
        ) : null}

        {rewindPreview ? (
          <div className="inlineNotice neutral">
            <div>
              <strong>Restore preview</strong>
              <p>{summarize(rewindPreview.restoredInput, 160)}</p>
              {rewindPreview.modifiedFiles.length > 0 ? (
                <p>Files: {rewindPreview.modifiedFiles.join(', ')}</p>
              ) : null}
              {rewindPreview.warnings.length > 0 ? <p>{rewindPreview.warnings.join(' | ')}</p> : null}
            </div>
            <div className="noticeActions">
              <button className="softButton" onClick={() => void rewind(true)} type="button">
                Restore files only
              </button>
              <button className="primaryAction" onClick={() => void rewind(false)} type="button">
                Confirm restore
              </button>
            </div>
          </div>
        ) : null}

        <div className="threadContent">
          {visibleTranscript.length === 0 ? <div className="emptyNote">No messages yet.</div> : null}
          {visibleTranscript.map((entry) => (
            <article className={`threadEntry ${entry.role}`} key={entry.id}>
              <div className="threadMeta">
                <strong>{roleLabel(entry.role)}</strong>
                <span>{relativeTime(entry.timestamp)}</span>
              </div>
              <div className="threadBody">
                <MarkdownBlock content={entry.content} />
              </div>
            </article>
          ))}
          {thinking ? (
            <article className="threadEntry assistant thinkingBlock">
              <div className="threadMeta">
                <strong>Thinking</strong>
                <span>live</span>
              </div>
              <div className="threadBody subtleText">{summarize(thinking, 220)}</div>
            </article>
          ) : null}
        </div>
      </section>
    );
  }

  function renderSettingsView() {
    return (
      <section className="surfacePage settingsPage">
        <div className="pageHeader">
          <div>
            <div className="eyebrow">Settings</div>
            <h2>Model configuration</h2>
          </div>
        </div>
        <div className="settingsForm">
          <label>
            Provider
            <input
              value={settings.api.provider}
              onChange={(event) =>
                setSettings((current) => ({
                  ...current,
                  api: { ...current.api, provider: event.target.value },
                }))
              }
            />
          </label>
          <label>
            Protocol
            <input
              value={settings.api.protocol}
              onChange={(event) =>
                setSettings((current) => ({
                  ...current,
                  api: { ...current.api, protocol: event.target.value },
                }))
              }
            />
          </label>
          <label>
            Base URL
            <input
              value={settings.api.base_url}
              onChange={(event) =>
                setSettings((current) => ({
                  ...current,
                  api: { ...current.api, base_url: event.target.value },
                }))
              }
            />
          </label>
          <label>
            API Key
            <input
              type="password"
              value={settings.api.api_key ?? ''}
              onChange={(event) =>
                setSettings((current) => ({
                  ...current,
                  api: { ...current.api, api_key: event.target.value || null },
                }))
              }
            />
          </label>
          <label>
            Model
            <input
              value={settings.model}
              onChange={(event) => setSettings((current) => ({ ...current, model: event.target.value }))}
            />
          </label>
        </div>
        <div className="pageActions">
          <button className="softButton" onClick={() => setView(currentSession ? 'thread' : 'home')} type="button">
            Back
          </button>
          <button className="primaryAction" disabled={saving} onClick={() => void saveCurrentSettings()} type="button">
            {saving ? 'Saving...' : 'Save settings'}
          </button>
        </div>
      </section>
    );
  }

  function renderAutomationView() {
    return (
      <section className="surfacePage automationPage">
        <div className="pageHeader">
          <div>
            <div className="eyebrow">Automation</div>
            <h2>Active tasks</h2>
          </div>
        </div>
        <div className="taskStack">
          {tasks.length === 0 ? <div className="emptyNote">No active tasks.</div> : null}
          {tasks.map((task) => (
            <article className="taskCard" key={task.id}>
              <div className="taskRow">
                <strong>{task.title}</strong>
                <span>{task.status}</span>
              </div>
              <small>
                {task.agentName} · {relativeTime(task.updatedAt)}
              </small>
              <p>{task.summary}</p>
            </article>
          ))}
        </div>
      </section>
    );
  }

  function renderHomeView() {
    return (
      <section className="homeView">
        <div className="heroMark">R</div>
        <h2>Start building</h2>
        <div className="heroProject">{projectName}</div>
        <div className="suggestionGrid">
          {SUGGESTIONS.map((item) => (
            <button key={item} className="suggestionCard" onClick={() => setComposer(item)} type="button">
              <span className="suggestionIcon">+</span>
              <span>{item}</span>
            </button>
          ))}
        </div>
      </section>
    );
  }

  function renderMain() {
    if (view === 'settings') return renderSettingsView();
    if (view === 'automation') return renderAutomationView();
    if (view === 'thread' && currentSession) return renderThreadView();
    return renderHomeView();
  }

  if (loading) {
    return <div className="screen loadingScreen">Loading desktop UI...</div>;
  }

  return (
    <div className="screen codexLikeApp">
      <aside className="navRail">
        <button className="navAction primary" onClick={() => void createSession()} type="button">
          New
        </button>
        <button className={`navAction ${view === 'home' ? 'active' : ''}`} onClick={() => setView('home')} type="button">
          Home
        </button>
        <button className={`navAction ${view === 'thread' ? 'active' : ''}`} onClick={() => setView('thread')} type="button">
          Chat
        </button>
        <button className={`navAction ${view === 'automation' ? 'active' : ''}`} onClick={() => setView('automation')} type="button">
          Tasks
        </button>
        <div className="navSpacer" />
        <button className={`navAction ${view === 'settings' ? 'active' : ''}`} onClick={() => setView('settings')} type="button">
          Settings
        </button>
      </aside>

      <aside className="threadPane">
        <div className="threadPaneHeader">
          <div>
            <div className="eyebrow">Workspace</div>
            <h1>{projectName}</h1>
          </div>
          <div className="threadPaneHeaderActions">
            <button className="iconButton" onClick={() => void openProjectFolder()} type="button">
              Reveal
            </button>
            <button className="iconButton" onClick={() => void chooseWorkingDirectory()} type="button">
              Select workspace
            </button>
            <button className="iconButton" onClick={() => void refreshSideData()} type="button">
              Refresh
            </button>
          </div>
        </div>

        <div className="threadListShell">
          {sessions.map((session) => (
            <div key={session.id} className={`threadListItem ${session.id === currentSession?.id ? 'active' : ''}`}>
              <button className="threadListMain" onClick={() => void restoreSession(session.id)} type="button">
                <strong>{session.name}</strong>
                <span>{summarize(sessionDetail(session), 48)}</span>
                <small>{relativeTime(session.updatedAt)}</small>
              </button>
              <button className="threadListDelete" onClick={() => void deleteSession(session.id)} type="button">
                Delete
              </button>
            </div>
          ))}
        </div>

        <div className="threadPaneFooter">
          <span className="threadPanePath" title={projectPath}>{projectPath}</span>
          <button className="softButton" onClick={() => void openProjectFolder()} type="button">
            Reveal
          </button>
        </div>
      </aside>

      <main className="mainSurface">
        <div className="topBar">
          <div className="topBarTitle">{view === 'thread' ? currentSession?.name ?? 'Session' : projectName}</div>
          <div className="topBarMeta">
            <span>{settings.model || 'No model selected'}</span>
            <span>{sending ? 'Generating' : 'Local workspace'}</span>
          </div>
        </div>

        <div className="surfaceContent">{renderMain()}</div>

        <div className="floatingComposerWrap">
          <div className="floatingComposer">
            <textarea
              placeholder="Ask RustCode anything, @ to add files, / for commands"
              value={composer}
              onChange={(event) => setComposer(event.target.value)}
              onKeyDown={onComposerKeyDown}
            />
            <div className="composerFooter">
              <div className="composerMeta">
                <span>Workspace mode</span>
                <span>Full access</span>
                <span>{thinking ? 'Thinking' : 'Ready'}</span>
              </div>
              <button className="sendFab" disabled={sending} onClick={() => void runPrompt()} type="button">
                Send
              </button>
            </div>
          </div>
        </div>
      </main>

      {showOnboarding ? (
        <div className="overlay">
          <div className="overlayCard">
            <div className="eyebrow">First run</div>
            <h2>Complete your model setup</h2>
            <p>Confirm provider, endpoint, model, and API key before continuing.</p>
            <button className="primaryAction" disabled={saving} onClick={() => void saveCurrentSettings(true)} type="button">
              {saving ? 'Saving...' : 'Finish setup'}
            </button>
          </div>
        </div>
      ) : null}

      {error ? <div className="errorToast">{error}</div> : null}

      <div className="activityDock">
        {activity.slice(0, 3).map((item) => (
          <div className="activityItem" key={item.id}>
            <span>{item.kind}</span>
            <p>{item.text}</p>
          </div>
        ))}
      </div>
    </div>
  );
}
