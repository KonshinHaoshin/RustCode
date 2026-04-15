import { useEffect, useMemo, useState, KeyboardEvent, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { BootstrapPayload, RestorePayload, SessionSummary, Settings, StreamPayload, SubmitPayload, TranscriptMessage } from './types';

// 核心字典：确保 en 和 zh 完全对称，防止 undefined 错误
const DICT = {
  zh: {
    recent: '最近会话', newChat: '新对话', thinking: '思考中', ready: '准备就绪',
    user: '你', assistant: '助手', settings: '设置中心', chat: '对话',
    placeholder: '输入消息或 / 触发命令...', profiles: '配置文件', active: '使用中',
    add: '新建配置', selectDir: '切换目录', plan: '规划模式', save: '保存更改',
    cancel: '放弃', edit: '编辑', 
    api: 'API 引擎', memory: '长期记忆', voice: '语音交互', rules: '权限管理',
    language: '界面语言'
  },
  en: {
    recent: 'Recent', newChat: 'New Chat', thinking: 'Thinking', ready: 'Ready',
    user: 'You', assistant: 'Assistant', settings: 'Settings', chat: 'Chat',
    placeholder: 'Ask anything or /...', profiles: 'Profiles', active: 'Active',
    add: 'New Profile', selectDir: 'Dir', plan: 'PLAN', save: 'Save',
    cancel: 'Cancel', edit: 'Edit', 
    api: 'API & Engine', memory: 'Memory', voice: 'Voice & UI', rules: 'Rules & Session',
    language: 'Language'
  }
};

const PROVIDERS = ['deepseek', 'openai', 'anthropic', 'xai', 'gemini', 'dashscope', 'openrouter', 'ollama', 'custom'];
const PROTOCOLS = ['open_ai', 'anthropic', 'responses'];

const SLASH_COMMANDS = [
  { cmd: '/init', desc: 'Initialize project' },
  { cmd: '/clear', desc: 'Clear history' },
  { cmd: '/model', desc: 'Switch model' },
  { cmd: '/fix', desc: 'Fix code issues' },
  { cmd: '/review', desc: 'Review changes' },
  { cmd: '/explain', desc: 'Explain code' },
  { cmd: '/test', desc: 'Generate tests' },
  { cmd: '/help', desc: 'Show help' }
];

function MarkdownRenderer({ content }: { content: string }) {
  const [copied, setCopied] = useState<number | null>(null);
  const copy = async (t: string, i: number) => { try { await navigator.clipboard.writeText(t); setCopied(i); setTimeout(() => setCopied(null), 2000); } catch (e) {} };
  
  const blocks = content.split(/(```[\s\S]*?```)/g);
  
  return (
    <div className="msg-body">
      {blocks.map((block, i) => {
        if (block.startsWith('```')) {
          const lines = block.split('\n');
          const lang = lines[0].replace('```', '').trim() || 'shell';
          const code = lines.slice(1, -1).join('\n');
          return (
            <div key={i} className="terminal-block">
              <div className="terminal-head">
                <div className="dot red" /><div className="dot yellow" /><div className="dot green" />
                <span className="lang-tag">{lang}</span>
                <button className="copy-btn" onClick={() => copy(code, i)}>{copied === i ? 'Done' : 'Copy'}</button>
              </div>
              <pre className="terminal-body"><code>{code}</code></pre>
            </div>
          );
        }

        const lines = block.split('\n');
        const rendered: React.ReactNode[] = [];
        let tableRows: string[] = [];

        const flushTable = (key: string) => {
          if (tableRows.length < 2) {
            tableRows.forEach((tr, tri) => rendered.push(<p key={`${key}-${tri}`} className="md-p">{tr}</p>));
            tableRows = [];
            return;
          }
          const headers = tableRows[0].split('|').filter(s => s.trim()).map(s => s.trim());
          const rows = tableRows.slice(2).filter(r => r.includes('|')).map(r => r.split('|').filter(s => s.trim()).map(s => s.trim()));
          rendered.push(
            <div key={key} className="table-wrapper" style={{ overflowX: 'auto', margin: '16px 0' }}>
              <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '13px' }}>
                <thead>
                  <tr>{headers.map((h, hi) => <th key={hi} style={{ border: '1px solid #eee', padding: '8px', background: '#f9f9f9', textAlign: 'left' }}>{h}</th>)}</tr>
                </thead>
                <tbody>
                  {rows.map((row, ri) => (
                    <tr key={ri}>{row.map((cell, ci) => <td key={ci} style={{ border: '1px solid #eee', padding: '8px' }}>{cell}</td>)}</tr>
                  ))}
                </tbody>
              </table>
            </div>
          );
          tableRows = [];
        };

        lines.forEach((line, li) => {
          if (line.includes('|')) {
            tableRows.push(line);
          } else {
            if (tableRows.length > 0) flushTable(`table-${i}-${li}`);
            const key = `${i}-${li}`;
            if (!line.trim()) rendered.push(<div key={key} style={{ height: '6px' }} />);
            else if (line.startsWith('# ')) rendered.push(<h1 key={key} className="md-h1">{line.slice(2)}</h1>);
            else if (line.startsWith('## ')) rendered.push(<h2 key={key} className="md-h2">{line.slice(3)}</h2>);
            else if (line.startsWith('### ')) rendered.push(<h3 key={key} className="md-h3">{line.slice(4)}</h3>);
            else if (line.startsWith('#### ')) rendered.push(<h4 key={key} className="md-h4">{line.slice(5)}</h4>);
            else {
              const parts = line.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
              rendered.push(
                <p key={key} className="md-p">
                  {parts.map((part, k) => {
                    if (part.startsWith('**')) return <strong key={k} className="md-bold">{part.slice(2, -2)}</strong>;
                    if (part.startsWith('`')) return <code key={k} className="inline-code">{part.slice(1, -1)}</code>;
                    return part;
                  })}
                </p>
              );
            }
          }
        });
        if (tableRows.length > 0) flushTable(`table-end-${i}`);
        return rendered;
      })}
    </div>
  );
}

export default function App() {
  const [lang, setLang] = useState<'en' | 'zh'>('zh');
  const t = DICT[lang];

  const [view, setView] = useState<'chat' | 'settings'>('chat');
  const [settingsTab, setSettingsTab] = useState<'api' | 'memory' | 'voice' | 'rules'>('api');
  
  const [projectName, setProjectName] = useState('');
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [currentSession, setCurrentSession] = useState<SessionSummary | null>(null);
  const [transcript, setTranscript] = useState<TranscriptMessage[]>([]);
  const [composer, setComposer] = useState('');
  const [sending, setSending] = useState(false);
  const [thinking, setThinking] = useState('');
  const [draftAssistant, setDraftAssistant] = useState('');
  const [showSlash, setShowSlash] = useState(false);
  const [planMode, setPlanMode] = useState(false);
  const [profiles, setProfiles] = useState<string[]>([]);
  const [activeProfile, setActiveProfile] = useState('');
  
  const [editingProfileName, setEditingProfileName] = useState<string | null>(null);
  const [editingSettings, setEditingSettings] = useState<Settings | null>(null);

  const flowRef = useRef<HTMLDivElement>(null);
  const draftBuffer = useRef("");

  useEffect(() => {
    const setupListeners = async () => {
      const unlistens = await Promise.all([
        listen<StreamPayload>('thinking_text_chunk', (e) => setThinking(p => p + (e.payload.delta ?? ''))),
        listen<StreamPayload>('assistant_text_chunk', (e) => {
          const delta = e.payload.delta ?? '';
          draftBuffer.current += delta;
          setDraftAssistant(draftBuffer.current);
        }),
        listen<StreamPayload>('turn_completed', () => {
          setTimeout(() => {
            setThinking('');
            setDraftAssistant('');
            draftBuffer.current = "";
            setSending(false);
            void bootstrap();
          }, 300);
        }),
      ]);
      return unlistens;
    };
    const promise = setupListeners();
    void bootstrap();
    void loadProfiles();
    return () => { void promise.then(u => u.forEach(fn => fn())); };
  }, []);

  useEffect(() => {
    if (flowRef.current) flowRef.current.scrollTop = flowRef.current.scrollHeight;
  }, [transcript, draftAssistant, thinking, view]);

  async function bootstrap() {
    try {
      const payload = await invoke<BootstrapPayload>('bootstrap_gui_state');
      setProjectName(payload.projectName);
      setSessions(payload.sessions);
      setCurrentSession(payload.currentSession);
      setTranscript(payload.transcript);
    } catch (e) {}
  }

  async function loadProfiles() {
    try {
      const list = await invoke<string[]>('list_profiles');
      const active = await invoke<string>('get_active_profile');
      setProfiles(list);
      setActiveProfile(active);
    } catch (e) {}
  }

  async function handleSwitchSession(id: string) {
    if (sending) return;
    try {
      await invoke('restore_session', { sessionId: id });
      void bootstrap();
      setView('chat');
    } catch (e) {}
  }

  async function handleSwitchProfile(name: string) {
    if (name === activeProfile || editingProfileName) return;
    try {
      const payload = await invoke<BootstrapPayload>('switch_profile', { name });
      setActiveProfile(name);
      setProjectName(payload.projectName);
      setSessions(payload.sessions);
      setCurrentSession(payload.currentSession);
      setTranscript(payload.transcript);
      setView('chat');
    } catch (e) {}
  }

  async function startEditing(name: string) {
    try {
      const settings = await invoke<Settings>('load_profile_settings', { name });
      setEditingProfileName(name);
      setEditingSettings(settings);
    } catch (e) { alert(e); }
  }

  async function saveEditing() {
    if (!editingProfileName || !editingSettings) return;
    try {
      await invoke('save_profile_settings', { name: editingProfileName, settings: editingSettings });
      setEditingProfileName(null);
      setEditingSettings(null);
      void loadProfiles();
      void bootstrap();
    } catch (e) { alert(e); }
  }

  const runPrompt = async (overridden?: string) => {
    const text = overridden || composer.trim();
    if (!text || sending) return;
    setSending(true); setComposer(''); setShowSlash(false); draftBuffer.current = "";
    try {
      const payload = await invoke<SubmitPayload>('submit_prompt', { prompt: text });
      setTranscript(payload.transcript);
      setCurrentSession(payload.session);
    } catch (e) { setSending(false); }
  };

  const visibleMessages = useMemo(() => {
    const msgs = [...transcript];
    if (draftAssistant) {
      msgs.push({ id: 'draft', role: 'assistant', content: draftAssistant, entry_type: 'message', parent_id: null, timestamp: new Date().toISOString() });
    }
    return msgs;
  }, [transcript, draftAssistant]);

  return (
    <div className="app-shell">
      <nav className="nav-rail">
        <div className={`rail-icon ${view === 'chat' ? 'active' : ''}`} onClick={() => setView('chat')}>💬</div>
        <div className={`rail-icon ${view === 'settings' ? 'active' : ''}`} onClick={() => setView('settings')}>⚙️</div>
        <div style={{ flex: 1 }} />
        <div className="rail-icon" onClick={() => void invoke('choose_working_directory').then(bootstrap)}>📂</div>
      </nav>

      {view === 'chat' ? (
        <>
          <aside className="threads-sidebar">
            <header className="sidebar-header">
              <span>{t.recent}</span>
              <div className="new-btn" onClick={() => void invoke('create_session').then(bootstrap)}>+</div>
            </header>
            <div className="thread-list">
              {sessions.map(s => (
                <div key={s.id} className={`thread-item ${s.id === currentSession?.id ? 'active' : ''}`} onClick={() => handleSwitchSession(s.id)}>
                  <div className="thread-name">{s.name}</div>
                  <div className="thread-meta">{s.message_count} msgs</div>
                </div>
              ))}
            </div>
          </aside>

          <main className="main-stage">
            <header className="stage-header">
              <div className="title">{currentSession?.name || projectName}</div>
              <div style={{ fontSize: '9px', fontWeight: 900, color: '#ccc' }}>{projectName.toUpperCase()}</div>
            </header>

            <section className="session-flow" ref={flowRef}>
              {visibleMessages.map(m => (
                <article key={m.id} className="msg-card">
                  <div className="msg-role">{m.role === 'user' ? t.user : t.assistant}</div>
                  <MarkdownRenderer content={m.content} />
                </article>
              ))}
              {thinking && (
                <div className="msg-card" style={{ opacity: 0.5 }}>
                  <div className="msg-role">{t.thinking}</div>
                  <div className="terminal-block" style={{ background: 'transparent', border: 'none', boxShadow: 'none' }}>
                    <div className="terminal-body" style={{ color: 'var(--text-sub)' }}>{thinking}</div>
                  </div>
                </div>
              )}
            </section>

            <div className="bottom-wrap">
              <div className="composer-container">
                {showSlash && (
                  <div className="slash-popover">
                    {SLASH_COMMANDS.map(c => (
                      <div key={c.cmd} className="slash-item" onClick={() => runPrompt(c.cmd)}>
                        <strong className="slash-cmd">{c.cmd}</strong>
                        <span className="slash-desc">{c.desc}</span>
                      </div>
                    ))}
                  </div>
                )}
                <textarea 
                  placeholder={t.placeholder} 
                  value={composer}
                  onChange={e => { setComposer(e.target.value); setShowSlash(e.target.value === '/'); }}
                  onKeyDown={e => e.key === 'Enter' && !e.shiftKey && (e.preventDefault(), runPrompt())}
                />
                <div className="composer-toolbar">
                  <div className="tool-btn-group">
                    <div className={`plan-pill ${planMode ? 'active' : ''}`} onClick={() => { setPlanMode(!planMode); runPrompt(!planMode ? '/plan on' : '/plan off'); }}>
                      <span>{t.plan}</span>
                    </div>
                  </div>
                  <div className="tool-btn-group">
                    <div style={{ fontSize: '9px', fontWeight: 800, color: '#ddd', marginRight: '8px' }}>{sending ? 'EXECUTING' : 'READY'}</div>
                    <button className="send-circle" onClick={() => runPrompt()} disabled={sending || !composer.trim()}>
                      <span style={{ fontSize: '14px' }}>↑</span>
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </main>
        </>
      ) : (
        <div style={{ display: 'flex', flex: 1 }}>
          <aside className="threads-sidebar" style={{ width: '220px' }}>
            <header className="sidebar-header"><span>{t.settings}</span></header>
            <div className="thread-list">
              <div className={`thread-item ${settingsTab === 'api' ? 'active' : ''}`} onClick={() => setSettingsTab('api')}>{t.api}</div>
              <div className={`thread-item ${settingsTab === 'memory' ? 'active' : ''}`} onClick={() => setSettingsTab('memory')}>{t.memory}</div>
              <div className={`thread-item ${settingsTab === 'voice' ? 'active' : ''}`} onClick={() => setSettingsTab('voice')}>{t.voice}</div>
              <div className={`thread-item ${settingsTab === 'rules' ? 'active' : ''}`} onClick={() => setSettingsTab('rules')}>{t.rules}</div>
            </div>
          </aside>

          <main className="main-stage">
            <header className="stage-header"><div className="title">Settings / {settingsTab.toUpperCase()}</div></header>
            <div className="settings-view">
              
              {settingsTab === 'api' && (
                <>
                  <div style={{ marginBottom: '32px' }}>
                    <h4 style={{ fontSize: '10px', color: '#bbb', marginBottom: '16px', letterSpacing: '1px' }}>CONFIGURATION PROFILES</h4>
                    <div className="settings-grid">
                      {profiles.map(name => (
                        <div key={name} className={`pro-card ${name === activeProfile ? 'active' : ''}`} onClick={() => handleSwitchProfile(name)}>
                          {name === activeProfile && <span className="tag">Active</span>}
                          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                            <div style={{ fontSize: '15px', fontWeight: 600 }}>{name}</div>
                            <button className="copy-btn" style={{ color: '#333', borderColor: '#eee' }} onClick={(e) => { e.stopPropagation(); startEditing(name); }}>{t.edit}</button>
                          </div>
                        </div>
                      ))}
                      <div className="pro-card" style={{ borderStyle: 'dashed', textAlign: 'center', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--accent)' }} onClick={() => { const n = window.prompt("Name:"); if(n) invoke('create_profile', {name: n}).then(loadProfiles); }}>+ {t.add}</div>
                    </div>
                  </div>

                  {editingSettings && (
                    <div className="pro-card active" style={{ cursor: 'default', marginTop: '40px' }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '20px' }}>
                        <strong style={{ fontSize: '16px' }}>Editing: {editingProfileName}</strong>
                        <div style={{ display: 'flex', gap: '8px' }}>
                          <button className="send-circle" style={{ borderRadius: '8px', width: 'auto', padding: '0 16px', fontSize: '11px', height: '28px', background: 'var(--accent)' }} onClick={saveEditing}>{t.save}</button>
                          <button className="send-circle" style={{ borderRadius: '8px', width: 'auto', padding: '0 16px', fontSize: '11px', height: '28px', background: '#f5f5f5', color: '#000' }} onClick={() => setEditingSettings(null)}>{t.cancel}</button>
                        </div>
                      </div>
                      <div className="settings-grid">
                        <div className="input-group"><label>Provider</label><select value={editingSettings.api.provider} onChange={e => setEditingSettings({...editingSettings, api: { ...editingSettings.api, provider: e.target.value }})}>{PROVIDERS.map(p => <option key={p} value={p}>{p}</option>)}</select></div>
                        <div className="input-group"><label>Protocol</label><select value={editingSettings.api.protocol} onChange={e => setEditingSettings({...editingSettings, api: { ...editingSettings.api, protocol: e.target.value }})}>{PROTOCOLS.map(p => <option key={p} value={p}>{p}</option>)}</select></div>
                        <div className="input-group"><label>Model</label><input value={editingSettings.model} onChange={e => setEditingSettings({...editingSettings, model: e.target.value})} /></div>
                        <div className="input-group"><label>API Key</label><input type="password" value={editingSettings.api.api_key || ''} onChange={e => setEditingSettings({...editingSettings, api: { ...editingSettings.api, api_key: e.target.value || null }})} /></div>
                        <div className="input-group" style={{ gridColumn: 'span 2' }}><label>Base URL</label><input value={editingSettings.api.base_url} onChange={e => setEditingSettings({...editingSettings, api: { ...editingSettings.api, base_url: e.target.value }})} /></div>
                      </div>
                    </div>
                  )}
                </>
              )}

              {settingsTab === 'voice' && (
                <div className="pro-card active" style={{ cursor: 'default' }}>
                  <div style={{ marginBottom: '32px' }}>
                    <h4 style={{ fontSize: '10px', color: '#bbb', marginBottom: '16px', letterSpacing: '1px' }}>GLOBAL APP SETTINGS</h4>
                    <div className="input-group">
                      <label>{t.language}</label>
                      <select value={lang} onChange={e => setLang(e.target.value as any)} style={{ maxWidth: '200px' }}>
                        <option value="en">English</option>
                        <option value="zh">简体中文</option>
                      </select>
                    </div>
                  </div>
                  {editingSettings && (
                    <div className="settings-grid">
                      <label style={{ display: 'flex', alignItems: 'center', gap: '12px' }}><input type="checkbox" checked={editingSettings.voice.enabled} onChange={e => setEditingSettings({...editingSettings, voice: {...editingSettings.voice, enabled: e.target.checked}})} /> Enable Voice</label>
                      <label style={{ display: 'flex', alignItems: 'center', gap: '12px' }}><input type="checkbox" checked={editingSettings.verbose} onChange={e => setEditingSettings({...editingSettings, verbose: e.target.checked})} /> Verbose Log</label>
                    </div>
                  )}
                </div>
              )}

              {settingsTab === 'memory' && editingSettings && (
                <div className="pro-card active" style={{ cursor: 'default' }}>
                  <h3>Memory Policy</h3>
                  <div className="settings-grid" style={{ marginTop: '20px' }}>
                    <label style={{ display: 'flex', alignItems: 'center', gap: '12px' }}><input type="checkbox" checked={editingSettings.memory.enabled} onChange={e => setEditingSettings({...editingSettings, memory: {...editingSettings.memory, enabled: e.target.checked}})} /> Enable Project Memory</label>
                    <div className="input-group"><label>Max Memories</label><input type="number" value={editingSettings.memory.max_memories} onChange={e => setEditingSettings({...editingSettings, memory: {...editingSettings.memory, max_memories: parseInt(e.target.value)}})} /></div>
                  </div>
                </div>
              )}

              {settingsTab === 'rules' && editingSettings && (
                <div className="pro-card active" style={{ cursor: 'default' }}>
                  <h3>Security & Session</h3>
                  <div className="settings-grid" style={{ marginTop: '20px' }}>
                    <div className="input-group"><label>Permission Mode</label><select value={editingSettings.permissions.mode} onChange={e => setEditingSettings({...editingSettings, permissions: { mode: e.target.value as any }})}><option value="ask">Ask</option><option value="allow_all">Allow All</option><option value="deny_all">Deny All</option></select></div>
                    <label style={{ display: 'flex', alignItems: 'center', gap: '12px' }}><input type="checkbox" checked={editingSettings.session.auto_restore_last_session} onChange={e => setEditingSettings({...editingSettings, session: {...editingSettings.session, auto_restore_last_session: e.target.checked}})} /> Auto Restore</label>
                  </div>
                </div>
              )}

            </div>
          </main>
        </div>
      )}
    </div>
  );
}
