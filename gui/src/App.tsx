import { useEffect, useMemo, useState, KeyboardEvent, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { BootstrapPayload, RestorePayload, SessionSummary, Settings, StreamPayload, SubmitPayload, TranscriptMessage } from './types';

const DICT = {
  zh: {
    recent: '最近会话', newChat: '新对话', thinking: '思考中', ready: '准备就绪',
    user: '你', assistant: '助手', settings: '设置', chat: '对话',
    placeholder: '输入消息或 / 触发命令...', profiles: '配置文件', active: '使用中',
    add: '新建配置', selectDir: '切换目录', plan: '规划模式', save: '保存',
    cancel: '取消', edit: '编辑', 
    api: 'API 引擎', memory: '长期记忆', voice: '语音交互', rules: '权限与会话',
    language: '界面语言', theme: '视觉主题'
  },
  en: {
    recent: 'Recent', newChat: 'New', thinking: 'Thinking', ready: 'Ready',
    user: 'You', assistant: 'Assistant', settings: 'Settings', chat: 'Chat',
    placeholder: 'Ask anything or /...', profiles: 'Profiles', active: 'Active',
    add: 'New Profile', selectDir: 'Dir', plan: 'PLAN', save: 'Save',
    cancel: 'Cancel', edit: 'Edit', 
    api: 'API & Engine', memory: 'Memory', voice: 'Voice & UI', rules: 'Rules & Session',
    language: 'Language', theme: 'Theme'
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
        return block.split('\n').map((line, j) => {
          if (!line.trim()) return <div key={`${i}-${j}`} style={{ height: '6px' }} />;
          const parts = line.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
          return (
            <p key={`${i}-${j}`} className="md-p">
              {parts.map((part, k) => {
                if (part.startsWith('**')) return <strong key={k} className="md-bold">{part.slice(2, -2)}</strong>;
                if (part.startsWith('`')) return <code key={k} className="inline-code">{part.slice(1, -1)}</code>;
                return part;
              })}
            </p>
          );
        });
      })}
    </div>
  );
}

export default function App() {
  const [lang, setLang] = useState<'en' | 'zh'>('en');
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

  useEffect(() => {
    const listeners: Promise<UnlistenFn>[] = [
      listen<StreamPayload>('thinking_text_chunk', (e) => setThinking(p => p + (e.payload.delta ?? ''))),
      listen<StreamPayload>('assistant_text_chunk', (e) => setDraftAssistant(p => p + (e.payload.delta ?? ''))),
      listen<StreamPayload>('turn_completed', () => { setThinking(''); setDraftAssistant(''); setSending(false); void bootstrap(); }),
    ];
    void bootstrap();
    void loadProfiles();
    return () => { void Promise.all(listeners).then(items => items.forEach(d => d())); };
  }, []);

  // 关键：自动滚动到底部
  useEffect(() => {
    if (flowRef.current) {
      flowRef.current.scrollTop = flowRef.current.scrollHeight;
    }
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
    setSending(true); setComposer(''); setShowSlash(false);
    try {
      const payload = await invoke<SubmitPayload>('submit_prompt', { prompt: text });
      setTranscript(payload.transcript);
      setCurrentSession(payload.session);
    } catch (e) { setSending(false); }
  };

  const visibleMessages = useMemo(() => {
    const msgs = [...transcript];
    if (draftAssistant) msgs.push({ id: 'draft', role: 'assistant', content: draftAssistant, entry_type: 'message', parent_id: null, timestamp: '' });
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
                <div key={s.id} className={`thread-item ${s.id === currentSession?.id ? 'active' : ''}`} onClick={() => void invoke('restore_session', { sessionId: s.id }).then(bootstrap)}>
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
            <header className="stage-header"><div className="title">{t.settings} / {settingsTab.toUpperCase()}</div></header>
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
                          <div style={{ fontSize: '11px', color: '#999', marginTop: '8px' }}>{name}.json</div>
                        </div>
                      ))}
                      <div className="pro-card" style={{ borderStyle: 'dashed', textAlign: 'center', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--accent)' }} onClick={() => { const n = window.prompt("Name:"); if(n) invoke('create_profile', {name: n}).then(loadProfiles); }}>
                        + {t.add}
                      </div>
                    </div>
                  </div>

                  {editingSettings && (
                    <div className="pro-card active" style={{ cursor: 'default', marginTop: '40px' }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '24px' }}>
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
                      <div style={{ display: 'flex', gap: '32px', marginTop: '20px', padding: '16px', background: '#fafafa', borderRadius: '12px' }}>
                        <label style={{ display: 'flex', alignItems: 'center', gap: '10px', fontSize: '13px', cursor: 'pointer' }}><input type="checkbox" checked={editingSettings.api.streaming} onChange={e => setEditingSettings({...editingSettings, api: { ...editingSettings.api, streaming: e.target.checked }})} /> Streaming Response</label>
                        <label style={{ display: 'flex', alignItems: 'center', gap: '10px', fontSize: '13px', cursor: 'pointer' }}><input type="checkbox" checked={editingSettings.verbose} onChange={e => setEditingSettings({...editingSettings, verbose: e.target.checked})} /> Verbose Developer Log</label>
                      </div>
                    </div>
                  )}
                </>
              )}

              {settingsTab === 'memory' && editingSettings && (
                <div className="pro-card active" style={{ cursor: 'default' }}>
                  <h3 style={{ marginBottom: '24px' }}>Long-term Memory Settings</h3>
                  <label style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '32px', fontSize: '14px', fontWeight: 600 }}>
                    <input type="checkbox" checked={editingSettings.memory.enabled} onChange={e => setEditingSettings({...editingSettings, memory: {...editingSettings.memory, enabled: e.target.checked}})} />
                    Enable Project-wide Context Memory
                  </label>
                  <div className="settings-grid">
                    <div className="input-group" style={{ gridColumn: 'span 2' }}><label>Storage Path</label><input value={editingSettings.memory.path} readOnly style={{ background: '#f9f9f9', color: '#888' }} /></div>
                    <div className="input-group"><label>Consolidation Interval (Hours)</label><input type="number" value={editingSettings.memory.consolidation_interval} onChange={e => setEditingSettings({...editingSettings, memory: {...editingSettings.memory, consolidation_interval: parseInt(e.target.value)}})} /></div>
                    <div className="input-group"><label>Memory Slot Capacity</label><input type="number" value={editingSettings.memory.max_memories} onChange={e => setEditingSettings({...editingSettings, memory: {...editingSettings.memory, max_memories: parseInt(e.target.value)}})} /></div>
                  </div>
                </div>
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
                    <>
                      <h3 style={{ marginBottom: '24px' }}>Voice & UI Interaction</h3>
                      <div className="settings-grid">
                        <label style={{ display: 'flex', alignItems: 'center', gap: '12px', fontSize: '14px' }}><input type="checkbox" checked={editingSettings.voice.enabled} onChange={e => setEditingSettings({...editingSettings, voice: {...editingSettings.voice, enabled: e.target.checked}})} /> Enable Voice Input</label>
                        <label style={{ display: 'flex', alignItems: 'center', gap: '12px', fontSize: '14px' }}><input type="checkbox" checked={editingSettings.voice.push_to_talk} onChange={e => setEditingSettings({...editingSettings, voice: {...editingSettings.voice, push_to_talk: e.target.checked}})} /> Push-to-Talk (PTT)</label>
                        <div className="input-group"><label>Microphone Sample Rate (Hz)</label><input type="number" value={editingSettings.voice.sample_rate} onChange={e => setEditingSettings({...editingSettings, voice: {...editingSettings.voice, sample_rate: parseInt(e.target.value)}})} /></div>
                        <div className="input-group"><label>Silence Threshold</label><input type="number" step="0.001" value={editingSettings.voice.silence_threshold} onChange={e => setEditingSettings({...editingSettings, voice: {...editingSettings.voice, silence_threshold: parseFloat(e.target.value)}})} /></div>
                      </div>
                    </>
                  )}
                </div>
              )}

              {settingsTab === 'rules' && editingSettings && (
                <div className="pro-card active" style={{ cursor: 'default' }}>
                  <h3 style={{ marginBottom: '24px' }}>Security Rules & Session Policy</h3>
                  <div className="input-group" style={{ marginBottom: '32px' }}>
                    <label>Global Permission Mode</label>
                    <select value={editingSettings.permissions.mode} onChange={e => setEditingSettings({...editingSettings, permissions: { mode: e.target.value as any }})} style={{ maxWidth: '400px' }}>
                      <option value="allow_all">Always Allow (High Risk)</option>
                      <option value="ask">Ask for Every Tool Call (Recommended)</option>
                      <option value="deny_all">Read-Only Mode (Deny All Writes)</option>
                    </select>
                  </div>
                  <div className="settings-grid" style={{ background: '#fafafa', padding: '20px', borderRadius: '16px' }}>
                    <label style={{ display: 'flex', alignItems: 'center', gap: '12px' }}><input type="checkbox" checked={editingSettings.session.auto_restore_last_session} onChange={e => setEditingSettings({...editingSettings, session: {...editingSettings.session, auto_restore_last_session: e.target.checked}})} /> Auto-restore Previous Session</label>
                    <label style={{ display: 'flex', alignItems: 'center', gap: '12px' }}><input type="checkbox" checked={editingSettings.session.persist_transcript} onChange={e => setEditingSettings({...editingSettings, session: {...editingSettings.session, persist_transcript: e.target.checked}})} /> Persist Chat Transcript to Disk</label>
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
