import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { BootstrapPayload, FileNode, RestorePayload, SessionSummary, StreamPayload, SubmitPayload, TranscriptMessage } from './types';

const DICT = {
  zh: {
    newChat: '新对话', recent: '最近会话', settings: '设置', workspace: '工作区',
    thinking: '思考中...', user: '你', assistant: 'RustCode', ready: '就绪',
    placeholder: '输入消息...', selectWorkspace: '切换目录'
  },
  en: {
    newChat: 'New Chat', recent: 'Recent', settings: 'Settings', workspace: 'Workspace',
    thinking: 'Thinking...', user: 'You', assistant: 'RustCode', ready: 'Ready',
    placeholder: 'Type a message...', selectWorkspace: 'Switch Dir'
  }
};

export default function App() {
  const [lang] = useState<'zh' | 'en'>('zh');
  const t = DICT[lang];

  const [projectName, setProjectName] = useState('');
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [currentSession, setCurrentSession] = useState<SessionSummary | null>(null);
  const [transcript, setTranscript] = useState<TranscriptMessage[]>([]);
  const [composer, setComposer] = useState('');
  const [sending, setSending] = useState(false);
  const [thinking, setThinking] = useState('');
  const [draftAssistant, setDraftAssistant] = useState('');

  useEffect(() => {
    const listeners: Promise<UnlistenFn>[] = [
      listen<StreamPayload>('thinking_text_chunk', (e) => setThinking(p => p + (e.payload.delta ?? ''))),
      listen<StreamPayload>('assistant_text_chunk', (e) => setDraftAssistant(p => p + (e.payload.delta ?? ''))),
      listen<StreamPayload>('turn_completed', () => { setThinking(''); setDraftAssistant(''); setSending(false); void bootstrap(); }),
    ];
    void bootstrap();
    return () => { void Promise.all(listeners).then(items => items.forEach(d => d())); };
  }, []);

  async function bootstrap() {
    try {
      const payload = await invoke<BootstrapPayload>('bootstrap_gui_state');
      setProjectName(payload.projectName);
      setSessions(payload.sessions);
      setCurrentSession(payload.currentSession);
      setTranscript(payload.transcript);
    } catch (e) { console.error(e); }
  }

  async function createNewSession() {
    const payload = await invoke<RestorePayload>('create_session');
    setCurrentSession(payload.session);
    setTranscript(payload.transcript);
    setComposer('');
    void bootstrap();
  }

  async function restoreSession(id: string) {
    const payload = await invoke<RestorePayload>('restore_session', { sessionId: id });
    setCurrentSession(payload.session);
    setTranscript(payload.transcript);
  }

  async function selectWorkspace() {
    const payload = await invoke<BootstrapPayload | null>('choose_working_directory');
    if (payload) {
      setProjectName(payload.projectName);
      setSessions(payload.sessions);
      setCurrentSession(payload.currentSession);
      setTranscript(payload.transcript);
    }
  }

  async function runPrompt() {
    if (!composer.trim() || sending) return;
    setSending(true);
    setComposer('');
    try {
      const payload = await invoke<SubmitPayload>('submit_prompt', { prompt: composer });
      setTranscript(payload.transcript);
    } catch (e) { setSending(false); }
  }

  const visibleTranscript = useMemo(() => {
    const items = [...transcript];
    if (draftAssistant) items.push({ id: 'draft', role: 'assistant', content: draftAssistant, entryType: 'message', parentId: null, timestamp: new Date().toISOString() });
    return items;
  }, [draftAssistant, transcript]);

  return (
    <div className="app-shell">
      {/* 1. Rail */}
      <nav className="nav-rail">
        <div className="rail-icon active">💬</div>
        <div className="rail-icon">📁</div>
        <div className="rail-icon">⚙️</div>
        <div style={{ flex: 1 }} />
        <div className="rail-icon" style={{ fontSize: '14px', fontWeight: 700 }} onClick={selectWorkspace}>📂</div>
      </nav>

      {/* 2. Sidebar */}
      <aside className="threads-sidebar">
        <header className="sidebar-header">
          <span>{t.recent}</span>
          <button style={{ color: 'var(--accent)', background: 'none', border: 'none', cursor: 'pointer', fontWeight: 700 }} onClick={createNewSession}>
            {t.newChat}
          </button>
        </header>
        <div className="thread-list">
          {sessions.map(s => (
            <div key={s.id} className={`thread-item ${s.id === currentSession?.id ? 'active' : ''}`} onClick={() => restoreSession(s.id)}>
              <div className="thread-name">{s.name}</div>
              <div className="thread-meta">{s.messageCount} msgs</div>
            </div>
          ))}
        </div>
      </aside>

      {/* 3. Stage */}
      <main className="main-stage">
        <header className="stage-header">
          <div style={{ fontWeight: 600 }}>{currentSession?.name || projectName}</div>
          <div style={{ fontSize: '11px', color: 'var(--text-sub)', fontWeight: 700 }}>{projectName.toUpperCase()}</div>
        </header>

        <section className="session-flow">
          {visibleTranscript.map(m => (
            <article key={m.id} className="msg-card">
              <div className="msg-role">{m.role === 'user' ? t.user : t.assistant}</div>
              <div className="msg-body" dangerouslySetInnerHTML={{ __html: m.content.replace(/\n/g, '<br/>') }} />
            </article>
          ))}
          {thinking && (
            <div className="msg-card" style={{ opacity: 0.6 }}>
              <div className="msg-role">{t.thinking}</div>
              <div className="msg-body">{thinking}</div>
            </div>
          )}
        </section>

        <div className="bottom-wrap">
          <div className="liquid-bar">
            <textarea 
              placeholder={t.placeholder} 
              value={composer}
              onChange={e => setComposer(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && !e.shiftKey && (e.preventDefault(), runPrompt())}
            />
            <div className="bar-footer">
              <div className="status-row">
                <span>{sending ? 'EXECUTING' : 'READY'}</span>
                <span onClick={selectWorkspace} style={{ cursor: 'pointer', textDecoration: 'underline' }}>{t.selectWorkspace}</span>
              </div>
              <button className="send-fab" onClick={runPrompt} disabled={sending}>
                {sending ? '...' : '↑'}
              </button>
            </div>
          </div>
        </div>
      </main>
    </div>
  );
}
