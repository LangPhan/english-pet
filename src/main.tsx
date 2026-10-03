import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { LogicalSize } from '@tauri-apps/api/dpi'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useMemo, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'

type Dashboard = {
  dueCount: number
  totalWords: number
  nextDueAt: string | null
  petPassthrough: boolean
}

type ReviewCard = {
  id: number
  term: string
  translation: string | null
  example: string | null
  dueAt: string
}

type Settings = {
  endpoint: string
  model: string
  desiredRetention: number
  quietHoursStart: number
  quietHoursEnd: number
  petPassthrough: boolean
  aiEnabled: boolean
  autostart: boolean
  hasApiKey: boolean
}

type Generated = { translation: string; example: string; exercise: string }
type Modal = 'capture' | 'review' | 'settings' | null

const initialSettings: Settings = {
  endpoint: 'http://localhost:20128/v1', model: '', desiredRetention: 0.9,
  quietHoursStart: 22, quietHoursEnd: 8, petPassthrough: false, aiEnabled: true, autostart: false, hasApiKey: false,
}

function App() {
  const [dashboard, setDashboard] = useState<Dashboard>({ dueCount: 0, totalWords: 0, nextDueAt: null, petPassthrough: false })
  const [modal, setModal] = useState<Modal>(null)
  const [term, setTerm] = useState('')
  const [translation, setTranslation] = useState('')
  const [example, setExample] = useState('')
  const [exercise, setExercise] = useState('')
  const [review, setReview] = useState<ReviewCard | null>(null)
  const [settings, setSettings] = useState<Settings>(initialSettings)
  const [apiKey, setApiKey] = useState('')
  const [models, setModels] = useState<string[]>([])
  const [dragging, setDragging] = useState(false)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [reminding, setReminding] = useState(false)
  const [showMeaning, setShowMeaning] = useState(false)
  const petRef = useRef<HTMLDivElement>(null)
  const importRef = useRef<HTMLInputElement>(null)

  const api = async <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args)
  const notify = (value: string) => {
    setMessage(value)
    window.setTimeout(() => setMessage(null), 3200)
  }
  const refresh = async () => {
    try { setDashboard(await api<Dashboard>('dashboard')) } catch (error) { notify(String(error)) }
  }
  const openCapture = async (clipboard = false) => {
    setTerm(''); setTranslation(''); setExample(''); setExercise('')
    if (clipboard) {
      try { setTerm(await api<string>('capture_clipboard')) } catch (error) { notify(String(error)) }
    }
    setModal('capture')
  }
  const openReview = async () => {
    try {
      const card = await api<ReviewCard | null>('next_review')
      setReview(card); setShowMeaning(false); setModal('review')
    } catch (error) { notify(String(error)) }
  }
  const openSettings = async () => {
    try { setSettings(await api<Settings>('get_settings')); setApiKey(''); setModal('settings') } catch (error) { notify(String(error)) }
  }

  useEffect(() => {
    const [width, height] = modal === 'settings' ? [580, 800] : modal ? [460, 620] : [220, 248]
    void getCurrentWindow().setSize(new LogicalSize(width, height))
  }, [modal])

  useEffect(() => {
    void refresh()
    const unlisten = Promise.all([
      listen('capture-clipboard', () => void openCapture(true)),
      listen('open-review', () => void openReview()),
      listen<number>('review-reminder', () => {
        setReminding(true)
        void refresh()
        window.setTimeout(() => setReminding(false), 30_000)
      }),
    ])
    return () => { void unlisten.then((items) => items.forEach((off) => off())) }
  }, [])

  const captureDrop = (event: React.DragEvent) => {
    event.preventDefault()
    setDragging(false)
    const dropped = event.dataTransfer.getData('text/plain').trim()
    if (!dropped) { notify('Không nhận được text. Copy từ rồi bấm Ctrl+Shift+E.'); return }
    setTerm(dropped); setTranslation(''); setExample(''); setExercise(''); setModal('capture')
  }
  const dragWindow = (event: React.PointerEvent) => {
    if ((event.target as HTMLElement).closest('button, input, textarea, select, .mouth')) return
    void getCurrentWindow().startDragging()
  }
  const saveWord = async () => {
    setBusy(true)
    try {
      await api('save_word', { input: { term, translation, example } })
      setModal(null); await refresh(); notify('Pet đã nuốt từ mới.')
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const generate = async () => {
    setBusy(true)
    try {
      const content = await api<Generated>('generate_content', { input: { term, translation: translation || null } })
      setTranslation(content.translation); setExample(content.example); setExercise(content.exercise)
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const rate = async (rating: number) => {
    if (!review) return
    setBusy(true)
    try {
      notify(await api<string>('rate_review', { cardId: review.id, rating }))
      await refresh()
      const next = await api<ReviewCard | null>('next_review')
      setReview(next); setShowMeaning(false)
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const saveSettings = async () => {
    setBusy(true)
    try {
      await api('save_settings', { input: { ...settings, apiKey: apiKey || undefined } })
      setModal(null); await refresh(); notify('Đã lưu Settings.')
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const loadModels = async () => {
    setBusy(true)
    try { setModels(await api<string[]>('ai_models')) } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const togglePassthrough = async () => {
    try {
      const active = await api<boolean>('toggle_passthrough')
      await refresh(); notify(active ? 'Pet click xuyên. Mở tray để mở khóa.' : 'Pet nhận chuột.')
    } catch (error) { notify(String(error)) }
  }
  const snooze = async (duration: 'hour' | 'tomorrow') => {
    try {
      await api<string>('snooze_reminder', { duration })
      setReminding(false)
      notify(duration === 'hour' ? 'Đã hoãn nhắc 1 giờ.' : 'Đã hoãn tới 8:00 sáng mai.')
    } catch (error) { notify(String(error)) }
  }
  const exportData = async () => {
    setBusy(true)
    try { notify(`Đã export JSON: ${await api<string>('export_data')}`) } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const backupSqlite = async () => {
    setBusy(true)
    try { notify(`Đã backup SQLite: ${await api<string>('backup_sqlite')}`) } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const importData = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    if (!window.confirm(`Import ${file.name}? Dữ liệu mới được thêm vào; từ trùng sẽ bị bỏ qua.`)) return
    setBusy(true)
    try {
      const result = await api<{ importedWords: number; skippedWords: number; importedReviews: number }>('import_data', { contents: await file.text() })
      await refresh()
      notify(`Đã import ${result.importedWords} từ, ${result.importedReviews} lượt ôn; bỏ qua ${result.skippedWords} từ trùng.`)
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const petLabel = useMemo(() => dashboard.dueCount > 0 ? `${dashboard.dueCount} từ chờ ôn` : 'Inbox trống', [dashboard.dueCount])

  return <main className="app" aria-live="polite">
    <section
      ref={petRef}
      className={`pet-shell ${dragging ? 'is-dragging' : ''} ${reminding ? 'is-reminding' : ''}`}
      onPointerDown={dragWindow}
      onDragEnter={(event) => { event.preventDefault(); setDragging(true) }}
      onDragOver={(event) => event.preventDefault()}
      onDragLeave={() => setDragging(false)}
      onDrop={captureDrop}
      aria-label="English Pet"
    >
      <div className="speech" role="status">
        <strong>{petLabel}</strong>
        <span>{dashboard.dueCount > 0 ? 'Bấm để ôn 1 phút' : 'Kéo từ vào miệng nhé'}</span>
      </div>
      <div className="pet" onDoubleClick={() => void openReview()}>
        <span className="ear ear-left" /><span className="ear ear-right" />
        <span className="eye eye-left" /><span className="eye eye-right" />
        <button className="mouth" title="Thêm từ từ clipboard" aria-label="Thêm từ từ clipboard" onClick={() => void openCapture(true)}>
          <span className="tongue" />
        </button>
        <span className="cheek cheek-left" /><span className="cheek cheek-right" />
      </div>
      {reminding && <div className="reminder-actions" aria-label="Hoãn nhắc ôn">
        <button onClick={() => void openReview()}>Ôn ngay</button>
        <button onClick={() => void snooze('hour')}>1 giờ</button>
        <button onClick={() => void snooze('tomorrow')}>Mai 8:00</button>
      </div>}
      <nav className="pet-actions" aria-label="Pet actions">
        <button onClick={() => void openReview()}>Ôn</button>
        <button onClick={() => void openCapture(true)}>+ Từ</button>
        <button onClick={() => void openSettings()} aria-label="Settings">⚙</button>
        <button onClick={() => void togglePassthrough()} title="Bật/tắt click xuyên">◎</button>
      </nav>
    </section>

    {message && <div className="toast" role="alert">{message}</div>}
    {modal === 'capture' && <Dialog title="Nuôi pet bằng từ mới" onClose={() => setModal(null)}>
      <p className="hint">Kéo text vào miệng pet, hoặc copy text rồi bấm <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>E</kbd>.</p>
      <Field label="Từ hoặc cụm từ"><input autoFocus value={term} onChange={(event) => setTerm(event.target.value)} placeholder="take care" maxLength={100} /></Field>
      <Field label="Nghĩa tiếng Việt"><input value={translation} onChange={(event) => setTranslation(event.target.value)} placeholder="chăm sóc" /></Field>
      <Field label="Ví dụ"><textarea value={example} onChange={(event) => setExample(event.target.value)} placeholder="Please take care of yourself." rows={3} /></Field>
      {exercise && <p className="exercise"><b>Bài tập AI:</b> {exercise}</p>}
      <footer><button className="secondary" disabled={!term || busy} onClick={() => void generate()}>✨ Tạo bằng AI</button><button className="primary" disabled={!term || busy} onClick={() => void saveWord()}>{busy ? 'Đang lưu…' : 'Cho pet ăn'}</button></footer>
    </Dialog>}

    {modal === 'review' && <Dialog title="Ôn cùng pet" onClose={() => setModal(null)}>
      {!review ? <div className="empty"><span>✦</span><h2>Hết từ đến hạn</h2><p>Pet nghỉ một chút. Thêm từ mới hoặc quay lại sau.</p></div> : <article className="review-card">
        <small>English → Vietnamese</small><h1>{review.term}</h1>
        {showMeaning ? <><p className="meaning">{review.translation || 'Chưa có nghĩa. Chỉnh sửa trong Inbox sau.'}</p>{review.example && <p className="example">“{review.example}”</p>}</> : <button className="reveal" onClick={() => setShowMeaning(true)}>Hiện nghĩa</button>}
        <div className="ratings">
          <button className="again" disabled={busy} onClick={() => void rate(1)}>Again</button>
          <button className="hard" disabled={busy} onClick={() => void rate(2)}>Hard</button>
          <button className="good" disabled={busy} onClick={() => void rate(3)}>Good</button>
          <button className="easy" disabled={busy} onClick={() => void rate(4)}>Easy</button>
        </div>
      </article>}
    </Dialog>}

    {modal === 'settings' && <Dialog title="Settings" onClose={() => setModal(null)} wide>
      <Field label="9Router endpoint"><input value={settings.endpoint} onChange={(event) => setSettings({ ...settings, endpoint: event.target.value })} /></Field>
      <Field label="Model"><div className="inline"><input value={settings.model} list="models" onChange={(event) => setSettings({ ...settings, model: event.target.value })} placeholder="Chọn hoặc nhập model ID" /><button className="secondary" disabled={busy} onClick={() => void loadModels()}>Tải models</button></div><datalist id="models">{models.map((model) => <option value={model} key={model} />)}</datalist></Field>
      <Field label="9Router API key"><input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={settings.hasApiKey ? 'Đã lưu an toàn. Nhập để thay hoặc xóa.' : 'Tùy chọn, chỉ nếu 9Router yêu cầu'} /></Field>
      <div className="setting-grid"><Field label="Desired retention"><input type="number" min="0.8" max="0.98" step="0.01" value={settings.desiredRetention} onChange={(event) => setSettings({ ...settings, desiredRetention: Number(event.target.value) })} /></Field><Field label="Giờ yên lặng từ"><input type="number" min="0" max="23" value={settings.quietHoursStart} onChange={(event) => setSettings({ ...settings, quietHoursStart: Number(event.target.value) })} /></Field><Field label="đến"><input type="number" min="0" max="23" value={settings.quietHoursEnd} onChange={(event) => setSettings({ ...settings, quietHoursEnd: Number(event.target.value) })} /></Field></div>
      <label className="check"><input type="checkbox" checked={settings.aiEnabled} onChange={(event) => setSettings({ ...settings, aiEnabled: event.target.checked })} /> Cho phép gửi từ đang thêm tới 9Router khi bấm “Tạo bằng AI”</label>
      <label className="check"><input type="checkbox" checked={settings.petPassthrough} onChange={(event) => setSettings({ ...settings, petPassthrough: event.target.checked })} /> Click xuyên pet (mở tray để bật lại tương tác)</label>
      <label className="check"><input type="checkbox" checked={settings.autostart} onChange={(event) => setSettings({ ...settings, autostart: event.target.checked })} /> Mở English Pet khi đăng nhập Windows</label>
      <section className="data-tools" aria-label="Sao lưu dữ liệu">
        <h3>Dữ liệu cục bộ</h3>
        <p>JSON dùng để chuyển dữ liệu. SQLite backup dùng để khôi phục toàn bộ dữ liệu học. Không gồm API key.</p>
        <div className="data-actions"><button className="secondary" disabled={busy} onClick={() => void exportData()}>Export JSON</button><button className="secondary" disabled={busy} onClick={() => void backupSqlite()}>Backup SQLite</button><button className="secondary" disabled={busy} onClick={() => importRef.current?.click()}>Import JSON</button></div>
        <input ref={importRef} className="visually-hidden" type="file" accept="application/json,.json" onChange={(event) => void importData(event)} />
      </section>
      <footer><button className="primary" disabled={busy} onClick={() => void saveSettings()}>{busy ? 'Đang lưu…' : 'Lưu Settings'}</button></footer>
    </Dialog>}
  </main>
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label className="field"><span>{label}</span>{children}</label>
}

function Dialog({ title, children, onClose, wide = false }: { title: string; children: React.ReactNode; onClose: () => void; wide?: boolean }) {
  return <div className="modal-backdrop" role="presentation" onMouseDown={onClose}><section className={`dialog ${wide ? 'wide' : ''}`} role="dialog" aria-modal="true" aria-label={title} onMouseDown={(event) => event.stopPropagation()}><header><h2>{title}</h2><button onClick={onClose} aria-label="Đóng">×</button></header>{children}</section></div>
}

createRoot(document.getElementById('root')!).render(<App />)
