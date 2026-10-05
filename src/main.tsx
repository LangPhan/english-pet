import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import corgiJump from './assets/corgi-jump.svg'
import corgiLie from './assets/corgi-lie.svg'
import corgiRun from './assets/corgi-run.svg'
import corgiRun2 from './assets/corgi-run-2.svg'
import corgiSit from './assets/corgi-sit.svg'
import corgiSleep from './assets/corgi-sleep.svg'
import './styles.css'

type Dashboard = {
  dueCount: number
  totalWords: number
  nextDueAt: string | null
  dueWord: string | null
  petPassthrough: boolean
}

type ReviewCard = {
  id: number
  term: string
  ipa: string | null
  translation: string | null
  example: string | null
  dueAt: string
}

type Settings = {
  endpoint: string
  model: string
  audioModel: string
  desiredRetention: number
  quietHoursStart: number
  quietHoursEnd: number
  petPassthrough: boolean
  petScale: number
  petOpacity: number
  aiEnabled: boolean
  autostart: boolean
  hasApiKey: boolean
}

type Generated = { translation: string; ipa: string | null; example: string; exercise: string; topic: Topic }
type PronunciationAssessment = { score: number; isCorrect: boolean; feedback: string; recognizedText: string; segments: { part: string; status: 'correct' | 'needs_improvement'; note: string | null }[] }
type Topic = 'daily_life' | 'work_tech' | 'study_academic' | 'travel_places' | 'food_dining' | 'health_sports' | 'entertainment' | 'emotions_social' | 'other'
type TopicFilter = Topic | 'unclassified' | ''
type VocabularyItem = { id: number; term: string; ipa: string | null; translation: string | null; example: string | null; topic: Topic | null; dueAt: string; createdAt: string }
type VocabularyPage = { items: VocabularyItem[]; total: number; unclassifiedCount: number }
type FeedPhase = 'flying' | 'chewing' | 'happy' | null
type PetPose = 'run' | 'jump' | 'sit' | 'lie' | 'sleep' | 'eat'
type Modal = 'review' | 'settings' | 'inbox' | 'edit' | null
type RoamStep = { hitEdge: boolean }
type ConfirmationRequest = {
  title: string
  description: React.ReactNode
  confirmLabel: string
  danger?: boolean
  onConfirm: () => Promise<void>
}
type LetterToken = { id: number; value: string }
type ChallengeStep = 'preview' | 'letters' | 'type' | 'complete'

const splitGraphemes = (value: string) => {
  const Segmenter = (Intl as typeof Intl & { Segmenter?: new (locale?: string, options?: { granularity: 'grapheme' }) => { segment: (text: string) => Iterable<{ segment: string }> } }).Segmenter
  return Segmenter ? Array.from(new Segmenter(undefined, { granularity: 'grapheme' }).segment(value), ({ segment }) => segment) : Array.from(value)
}
const shuffledTokens = (term: string): LetterToken[] => {
  const tokens = splitGraphemes(term).filter((character) => !/\s/.test(character)).map((value, id) => ({ id, value }))
  for (let index = tokens.length - 1; index > 0; index--) {
    const other = Math.floor(Math.random() * (index + 1))
    ;[tokens[index], tokens[other]] = [tokens[other], tokens[index]]
  }
  return tokens
}
const normalizeAnswer = (value: string) => value.trim().replace(/\s+/g, ' ').toLocaleLowerCase()

const topics: Record<Topic, string> = {
  daily_life: 'Đời sống', work_tech: 'Công việc & Công nghệ', study_academic: 'Học tập', travel_places: 'Du lịch', food_dining: 'Ẩm thực',
  health_sports: 'Sức khỏe', entertainment: 'Giải trí', emotions_social: 'Giao tiếp & Cảm xúc', other: 'Khác',
}

const initialSettings: Settings = {
  endpoint: 'http://localhost:20128/v1', model: '', audioModel: '', desiredRetention: 0.9,
  quietHoursStart: 22, quietHoursEnd: 8, petPassthrough: false, petScale: 1, petOpacity: 1, aiEnabled: true, autostart: false, hasApiKey: false,
}

async function audioBlobToWavBase64(blob: Blob): Promise<string> {
  const arrayBuffer = await blob.arrayBuffer()
  const AudioCtx = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
  const audioContext = new AudioCtx({ sampleRate: 16000 })
  try {
    const audioBuffer = await audioContext.decodeAudioData(arrayBuffer)
    const pcmData = audioBuffer.getChannelData(0)
    const wavBuffer = new ArrayBuffer(44 + pcmData.length * 2)
    const view = new DataView(wavBuffer)
    const writeString = (offset: number, str: string) => {
      for (let i = 0; i < str.length; i++) view.setUint8(offset + i, str.charCodeAt(i))
    }
    writeString(0, 'RIFF')
    view.setUint32(4, 36 + pcmData.length * 2, true)
    writeString(8, 'WAVE')
    writeString(12, 'fmt ')
    view.setUint32(16, 16, true)
    view.setUint16(20, 1, true)
    view.setUint16(22, 1, true)
    view.setUint32(24, 16000, true)
    view.setUint32(28, 32000, true)
    view.setUint16(32, 2, true)
    view.setUint16(34, 16, true)
    writeString(36, 'data')
    view.setUint32(40, pcmData.length * 2, true)

    let offset = 44
    for (let i = 0; i < pcmData.length; i++, offset += 2) {
      const s = Math.max(-1, Math.min(1, pcmData[i]))
      view.setInt16(offset, s < 0 ? s * 0x8000 : s * 0x7fff, true)
    }

    const bytes = new Uint8Array(wavBuffer)
    let binary = ''
    const chunk = 8192
    for (let i = 0; i < bytes.length; i += chunk) {
      binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunk)))
    }
    return btoa(binary)
  } finally {
    void audioContext.close()
  }
}

function App() {
  const [dashboard, setDashboard] = useState<Dashboard>({ dueCount: 0, totalWords: 0, nextDueAt: null, dueWord: null, petPassthrough: false })
  const [modal, setModal] = useState<Modal>(null)
  const modalRef = useRef<Modal>(null)
  modalRef.current = modal
  const [term, setTerm] = useState('')
  const [ipa, setIpa] = useState('')
  const [translation, setTranslation] = useState('')
  const [example, setExample] = useState('')
  const [exercise, setExercise] = useState('')
  const [topic, setTopic] = useState<Topic | ''>('')
  const [editingWord, setEditingWord] = useState<VocabularyItem | null>(null)
  const [feedingWord, setFeedingWord] = useState<string | null>(null)
  const [feedPhase, setFeedPhase] = useState<FeedPhase>(null)
  const [review, setReview] = useState<ReviewCard | null>(null)
  const [vocabulary, setVocabulary] = useState<VocabularyPage>({ items: [], total: 0, unclassifiedCount: 0 })
  const [search, setSearch] = useState('')
  const [topicFilter, setTopicFilter] = useState<TopicFilter>('')
  const [vocabularyOffset, setVocabularyOffset] = useState(0)
  const [settings, setSettings] = useState<Settings>(initialSettings)
  const [apiKey, setApiKey] = useState('')
  const [models, setModels] = useState<string[]>([])
  const [dragging, setDragging] = useState(false)
  const [petHovering, setPetHovering] = useState(false)
  const [petPose, setPetPose] = useState<PetPose>('sit')
  const [petDirection, setPetDirection] = useState<1 | -1>(-1)
  const petDirectionRef = useRef<1 | -1>(-1)
  const petJumpingRef = useRef(false)
  const petRoamQueueRef = useRef<Promise<void>>(Promise.resolve())
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [reminding, setReminding] = useState(false)
  const [showMeaning, setShowMeaning] = useState(false)
  const [challengeStep, setChallengeStep] = useState<ChallengeStep>('letters')
  const [letterTokens, setLetterTokens] = useState<LetterToken[]>([])
  const [selectedTokenIds, setSelectedTokenIds] = useState<number[]>([])
  const [typedAnswer, setTypedAnswer] = useState('')
  const [challengeFeedback, setChallengeFeedback] = useState<string | null>(null)
  const typeAnswerRef = useRef<HTMLInputElement>(null)
  const [practiceOpen, setPracticeOpen] = useState(false)
  const [recording, setRecording] = useState(false)
  const [recordedAudio, setRecordedAudio] = useState<string | null>(null)
  const [recordedBlob, setRecordedBlob] = useState<Blob | null>(null)
  const [assessment, setAssessment] = useState<PronunciationAssessment | null>(null)
  const [confirmation, setConfirmation] = useState<ConfirmationRequest | null>(null)
  const [confirming, setConfirming] = useState(false)
  const recorderRef = useRef<MediaRecorder | null>(null)
  const confirmingRef = useRef(false)
  const streamRef = useRef<MediaStream | null>(null)
  const recordTimerRef = useRef<number | null>(null)
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
  const speak = (text: string) => {
    if (!('speechSynthesis' in window)) { notify('Windows không hỗ trợ đọc mẫu trên thiết bị này.'); return }
    window.speechSynthesis.cancel()
    const utterance = new SpeechSynthesisUtterance(text)
    utterance.lang = 'en-US'
    utterance.rate = 0.9 // ponytail: use Windows default English voice; add voice picker only on request.
    window.speechSynthesis.speak(utterance)
  }
  const clearRecording = () => {
    if (recordTimerRef.current) window.clearTimeout(recordTimerRef.current)
    recordTimerRef.current = null
    if (recorderRef.current && recorderRef.current.state === 'recording') {
      recorderRef.current.stop()
    }
    recorderRef.current = null
    streamRef.current?.getTracks().forEach((track) => track.stop())
    streamRef.current = null
    if (recordedAudio) URL.revokeObjectURL(recordedAudio)
    setRecordedAudio(null); setRecordedBlob(null); setRecording(false); setAssessment(null)
  }
  const startRecording = async () => {
    if (!navigator.mediaDevices?.getUserMedia) {
      notify('Thiết bị không hỗ trợ microphone.')
      return
    }
    clearRecording()
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      streamRef.current = stream
      const chunks: BlobPart[] = []
      const recorder = new MediaRecorder(stream)
      recorderRef.current = recorder
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunks.push(event.data)
      }
      recorder.onstop = () => {
        stream.getTracks().forEach((track) => track.stop())
        streamRef.current = null
        const blob = new Blob(chunks, { type: recorder.mimeType || 'audio/webm' })
        setRecordedBlob(blob)
        setRecordedAudio(URL.createObjectURL(blob))
        setRecording(false)
      }
      recorder.start()
      setRecording(true)
      recordTimerRef.current = window.setTimeout(() => {
        if (recorderRef.current && recorderRef.current.state === 'recording') {
          recorderRef.current.stop()
        }
      }, 10_000)
    } catch (error) {
      notify('Không thể mở microphone: ' + String(error))
      clearRecording()
    }
  }
  const stopRecording = () => {
    if (recordTimerRef.current) {
      window.clearTimeout(recordTimerRef.current)
      recordTimerRef.current = null
    }
    if (recorderRef.current && recorderRef.current.state === 'recording') {
      recorderRef.current.stop()
    }
  }
  const scorePronunciation = async () => {
    if (!review || !recordedBlob) return
    setBusy(true)
    try {
      const base64 = await audioBlobToWavBase64(recordedBlob)
      const res = await api<PronunciationAssessment>('evaluate_pronunciation', {
        input: { cardId: review.id, audioBase64: base64 }
      })
      setAssessment(res)
    } catch (error) {
      notify(String(error))
    } finally {
      setBusy(false)
    }
  }
  const resetChallenge = (card: ReviewCard | null) => {
    setChallengeStep('preview')
    setLetterTokens(card ? shuffledTokens(card.term) : [])
    setSelectedTokenIds([])
    setTypedAnswer('')
    setChallengeFeedback(null)
  }
  const chooseLetter = (token: LetterToken) => {
    if (!review || challengeStep !== 'letters') return
    const expected = splitGraphemes(review.term).filter((character) => !/\s/.test(character))
    const selected = [...selectedTokenIds, token.id]
    if (token.value !== expected[selectedTokenIds.length]) {
      setSelectedTokenIds([])
      setChallengeFeedback('Chưa đúng thứ tự. Thử lại từ đầu nhé.')
      return
    }
    setSelectedTokenIds(selected)
    setChallengeFeedback(null)
    if (selected.length === expected.length) {
      setChallengeStep('type')
      window.setTimeout(() => typeAnswerRef.current?.focus(), 0)
    }
  }
  const checkTypedAnswer = () => {
    if (!review || challengeStep !== 'type') return
    if (normalizeAnswer(typedAnswer) !== normalizeAnswer(review.term)) {
      setChallengeFeedback('Chưa đúng. Kiểm tra lại chính tả rồi thử tiếp.')
      typeAnswerRef.current?.focus()
      return
    }
    setChallengeFeedback('✓ Hoàn thành hai thử thách. Bạn có thể chọn mức ôn.')
    setChallengeStep('complete')
  }
  const feedWord = async (rawTerm?: string) => {
    if (feedingWord) return
    try {
      const capturedTerm = rawTerm?.trim() || await api<string>('capture_clipboard')
      if (!capturedTerm) return
      setFeedingWord(capturedTerm); setFeedPhase('flying')
      window.setTimeout(async () => {
        setFeedPhase('chewing')
        try {
          await api('save_word', { input: { term: capturedTerm, ipa: null, translation: null, example: null, topic: null } })
          await refresh()
          window.setTimeout(() => {
            speak(capturedTerm)
            setFeedPhase('happy')
            window.setTimeout(() => { setFeedPhase(null); setFeedingWord(null) }, 2_000)
          }, 900)
        } catch (error) {
          notify(String(error)); setFeedPhase(null); setFeedingWord(null)
        }
      }, 700)
    } catch (error) { notify(String(error)) }
  }
  const openEdit = (word: VocabularyItem) => {
    setEditingWord(word); setTerm(word.term); setIpa(word.ipa || ''); setTranslation(word.translation || ''); setExample(word.example || ''); setExercise(''); setTopic(word.topic || ''); setModal('edit')
  }
  const openReview = async () => {
    try {
      clearRecording()
      const card = await api<ReviewCard | null>('next_review')
      setReview(card); resetChallenge(card); setShowMeaning(false); setPracticeOpen(false); setModal('review')
    } catch (error) { notify(String(error)) }
  }
  const openSettings = async () => {
    try { setSettings(await api<Settings>('get_settings')); setApiKey(''); setModal('settings') } catch (error) { notify(String(error)) }
  }
  const loadVocabulary = async (nextSearch = search, nextTopic = topicFilter, nextOffset = vocabularyOffset) => {
    try {
      setVocabulary(await api<VocabularyPage>('list_vocabulary', { input: { search: nextSearch || null, topic: nextTopic || null, offset: nextOffset } }))
      setVocabularyOffset(nextOffset)
    } catch (error) { notify(String(error)) }
  }
  const openInbox = async () => {
    setSearch(''); setTopicFilter(''); setVocabularyOffset(0); setModal('inbox')
    await loadVocabulary('', '', 0)
  }

  useEffect(() => {
    void invoke('set_window_mode', { mode: modal ?? 'pet' }).catch((error) => notify(String(error)))
    if (modal !== 'review') clearRecording()
  }, [modal])

  useEffect(() => () => clearRecording(), [])

  const stepPetRoam = (deltaX: number, jumpOffset: number) => {
    const step = () => api<RoamStep>('step_pet_roam', { deltaX, jumpOffset })
    const result = petRoamQueueRef.current.then(step, step)
    petRoamQueueRef.current = result.then(() => undefined, () => undefined)
    return result
  }
  const resetPetJump = async () => {
    if (!petJumpingRef.current || modalRef.current) return
    petJumpingRef.current = false
    await stepPetRoam(0, 0)
  }

  const hasDueWord = !feedingWord && Boolean(dashboard.dueWord)

  useEffect(() => {
    if (modal || petHovering || dragging || feedPhase || hasDueWord || settings.petPassthrough) {
      setPetPose(feedPhase || hasDueWord ? 'eat' : 'sit')
      return
    }
    let cancelled = false
    let timer = 0
    const nextPose = (): Exclude<PetPose, 'sit' | 'eat'> => {
      const roll = Math.random()
      return roll < 0.5 ? 'run' : roll < 0.67 ? 'jump' : roll < 0.84 ? 'lie' : 'sleep'
    }
    const run = async () => {
      const pose = nextPose()
      setPetPose(pose)
      const duration = pose === 'run' ? 3_000 + Math.random() * 4_000 : pose === 'jump' ? 650 : pose === 'lie' ? 4_000 + Math.random() * 3_000 : 8_000 + Math.random() * 5_000
      const started = Date.now()
      const tick = async () => {
        if (cancelled) return
        if (pose === 'run' || pose === 'jump') {
          try {
            if (pose === 'jump') petJumpingRef.current = true
            const result = await stepPetRoam(
              petDirectionRef.current * (pose === 'run' ? 4 : 2),
              pose === 'jump' ? 12 : 0,
            )
            if (cancelled) return
            if (result.hitEdge) {
              petDirectionRef.current = petDirectionRef.current === 1 ? -1 : 1
              setPetDirection(petDirectionRef.current)
            }
          } catch (error) {
            petJumpingRef.current = false
            if (!cancelled) notify(String(error))
            return
          }
        }
        if (Date.now() - started >= duration) {
          if (pose === 'jump') {
            try {
              await resetPetJump()
            } catch (error) {
              if (!cancelled) notify(String(error))
              return
            }
          }
          if (cancelled) return
          setPetPose('sit')
          timer = window.setTimeout(() => void run(), 1_500)
        } else {
          timer = window.setTimeout(() => void tick(), pose === 'run' ? 80 : 100)
        }
      }
      void tick()
    }
    timer = window.setTimeout(() => void run(), 2_000)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
      void resetPetJump()
    }
  }, [modal, petHovering, dragging, feedPhase, hasDueWord, settings.petPassthrough])

  useEffect(() => {
    void refresh()
    const unlisten = Promise.all([
      listen<string>('capture-selection', ({ payload }) => void feedWord(payload)),
      listen<string>('capture-error', ({ payload }) => notify(payload)),
      listen('capture-clipboard', () => void feedWord()),
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
    if (!dropped) { notify('Không nhận được text. Bôi đen text rồi bấm Ctrl+Shift+E.'); return }
    void feedWord(dropped)
  }
  const dragWindow = (event: React.PointerEvent) => {
    if ((event.target as HTMLElement).closest('button, input, textarea, select')) return
    void getCurrentWindow().startDragging()
  }
  const saveWord = async () => {
    if (!editingWord) return
    setBusy(true)
    try {
      await api('update_word', { input: { id: editingWord.id, ipa: ipa || null, translation, example: example || null, topic: topic || null } })
      setModal('inbox'); await loadVocabulary(); await refresh(); notify('Đã định nghĩa từ.')
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const generate = async () => {
    setBusy(true)
    try {
      const content = await api<Generated>('generate_content', { input: { term, translation: translation || null } })
      setTranslation(content.translation); setIpa(content.ipa || ''); setExample(content.example); setExercise(content.exercise); setTopic(content.topic)
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const defineWithAi = async (word: VocabularyItem) => {
    setBusy(true)
    try {
      const content = await api<Generated>('generate_content', { input: { term: word.term, translation: word.translation || null } })
      await api('update_word', { input: { id: word.id, ipa: content.ipa, translation: content.translation, example: content.example, topic: content.topic } })
      await loadVocabulary(); await refresh(); notify(`AI đã định nghĩa “${word.term}”.`)
    } catch (error) { notify(String(error)) } finally { setBusy(false) }
  }
  const requestConfirmation = (request: ConfirmationRequest) => setConfirmation(request)
  const confirm = async () => {
    if (!confirmation || confirmingRef.current) return
    confirmingRef.current = true
    setConfirming(true)
    try {
      await confirmation.onConfirm()
      setConfirmation(null)
    } finally {
      confirmingRef.current = false
      setConfirming(false)
    }
  }
  const deletePendingWord = (word: VocabularyItem) => requestConfirmation({
    title: 'Xóa từ chờ?',
    description: <>“{word.term}” sẽ bị xóa khỏi Inbox. Từ đã định nghĩa không thể xóa ở đây.</>,
    confirmLabel: 'Xóa từ',
    danger: true,
    onConfirm: async () => {
      setBusy(true)
      try {
        await api('delete_pending_word', { id: word.id })
        await loadVocabulary(); await refresh(); notify(`Đã xóa “${word.term}”.`)
      } catch (error) { notify(String(error)) } finally { setBusy(false) }
    },
  })
  const rate = async (rating: number) => {
    if (!review) return
    clearRecording()
    setPracticeOpen(false)
    setBusy(true)
    try {
      notify(await api<string>('rate_review', { cardId: review.id, rating }))
      await refresh()
      const next = await api<ReviewCard | null>('next_review')
      setReview(next); resetChallenge(next); setShowMeaning(false)
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
  const importData = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    requestConfirmation({
      title: 'Import dữ liệu?',
      description: <><b>{file.name}</b> sẽ thêm từ và lịch sử ôn mới. Từ trùng sẽ được bỏ qua, dữ liệu hiện có không bị thay thế.</>,
      confirmLabel: 'Import',
      onConfirm: async () => {
        setBusy(true)
        try {
          const result = await api<{ importedWords: number; skippedWords: number; importedReviews: number }>('import_data', { contents: await file.text() })
          await refresh()
          notify(`Đã import ${result.importedWords} từ, ${result.importedReviews} lượt ôn; bỏ qua ${result.skippedWords} từ trùng.`)
        } catch (error) { notify(String(error)) } finally { setBusy(false) }
      },
    })
  }
  const definePendingWithAi = () => {
    const ids = vocabulary.items.filter((word) => !word.translation).map((word) => word.id)
    if (!ids.length) return
    const endpoint = settings.endpoint
    requestConfirmation({
      title: 'Gửi từ tới AI?',
      description: <>{ids.length} từ chờ định nghĩa đang xem sẽ được gửi tới <b>{endpoint}</b> để tạo nghĩa, IPA, ví dụ và chủ đề. Không gửi từ khác hay lịch sử ôn.</>,
      confirmLabel: 'Gửi tới AI',
      onConfirm: async () => {
        setBusy(true)
        try {
          const result = await api<{ updatedCount: number; failedCount: number }>('batch_define_vocabulary', { input: { ids } })
          await loadVocabulary(); await refresh()
          notify(`AI đã định nghĩa ${result.updatedCount} từ${result.failedCount ? `; ${result.failedCount} từ chưa xong` : ''}.`)
        } catch (error) { notify(String(error)) } finally { setBusy(false) }
      },
    })
  }
  return <main className="app" aria-live="polite">
    <section
      ref={petRef}
      className={`pet-shell ${petHovering ? 'is-hovering' : ''} ${dragging ? 'is-dragging' : ''} ${reminding ? 'is-reminding' : ''} ${hasDueWord ? 'is-due' : ''} ${feedPhase ? `is-${feedPhase}` : ''}`}
      style={{ '--pet-opacity': settings.petOpacity, '--pet-scale': settings.petScale } as React.CSSProperties}
      onPointerDown={dragWindow}
      onPointerEnter={() => setPetHovering(true)}
      onPointerLeave={() => setPetHovering(false)}
      onFocusCapture={() => setPetHovering(true)}
      onBlurCapture={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node)) setPetHovering(false) }}
      onDragEnter={(event) => { event.preventDefault(); setDragging(true) }}
      onDragOver={(event) => event.preventDefault()}
      onDragLeave={() => setDragging(false)}
      onDrop={captureDrop}
      aria-label="English Pet"
    >
      {feedingWord && <span className="bone" aria-hidden="true"><span>{feedingWord}</span></span>}
      {hasDueWord && <span className="bone due-bone" aria-hidden="true"><span>{dashboard.dueWord}</span></span>}
      <button className="pixel-corgi" data-pose={petPose} data-facing={petDirection === -1 ? 'left' : 'right'} aria-label={hasDueWord ? `Ôn từ: ${dashboard.dueWord}` : 'Ôn từ đến hạn'} onClick={() => void openReview()}>
        {petPose === 'run' ? <><img className="pixel-frame-1" src={corgiRun} alt="" /><img className="pixel-frame-2" src={corgiRun2} alt="" /></> : <img src={petPose === 'jump' ? corgiJump : petPose === 'lie' || petPose === 'eat' ? corgiLie : petPose === 'sleep' ? corgiSleep : corgiSit} alt="" />}
      </button>
      {reminding && <div className="reminder-actions" aria-label="Hoãn nhắc ôn">
        <button onClick={() => void openReview()}>Ôn ngay</button>
        <button onClick={() => void snooze('hour')}>1 giờ</button>
        <button onClick={() => void snooze('tomorrow')}>Mai 8:00</button>
      </div>}
      <nav className="pet-actions" aria-label="Chức năng pet">
        <button data-tooltip="Ôn từ đến hạn" aria-label="Ôn từ đến hạn" onClick={() => void openReview()}>📚</button>
        <button data-tooltip="Thêm từ" aria-label="Thêm từ" disabled={Boolean(feedingWord)} onClick={() => void feedWord()}>➕</button>
        <button data-tooltip="Xem từ đã thêm" aria-label="Xem từ đã thêm" onClick={() => void openInbox()}>📖</button>
        <button data-tooltip="Settings" aria-label="Settings" onClick={() => void openSettings()}>⚙</button>
        <button data-tooltip="Bật/tắt click xuyên" aria-label="Bật/tắt click xuyên" onClick={() => void togglePassthrough()}>◎</button>
      </nav>
    </section>

    {message && <div className="toast" role="alert">{message}</div>}
    {confirmation && <ConfirmationDialog request={confirmation} running={confirming} onCancel={() => setConfirmation(null)} onConfirm={() => void confirm()} />}
    {modal === 'edit' && editingWord && <Dialog title={`Định nghĩa: ${editingWord.term}`} onClose={() => setModal('inbox')}>
      <div className="term-heading">
        <h3>{editingWord.term}</h3>
        <button className="speaker-button" type="button" aria-label={`Nghe phát âm ${editingWord.term}`} onClick={() => speak(editingWord.term)}>🔊</button>
      </div>
      <Field label="Phát âm (IPA)"><input value={ipa} onChange={(event) => setIpa(event.target.value)} placeholder="/.../" maxLength={200} /></Field>
      <Field label="Nghĩa tiếng Việt"><input autoFocus value={translation} onChange={(event) => setTranslation(event.target.value)} placeholder="chăm sóc" maxLength={2000} /></Field>
      <Field label="Ví dụ"><textarea value={example} onChange={(event) => setExample(event.target.value)} placeholder="Please take care of yourself." rows={3} maxLength={5000} /></Field>
      <Field label="Chủ đề"><select value={topic} onChange={(event) => setTopic(event.target.value as Topic | '')}><option value="">Chưa phân loại</option>{Object.entries(topics).map(([id, label]) => <option value={id} key={id}>{label}</option>)}</select></Field>
      {exercise && <p className="exercise"><b>Bài tập AI:</b> {exercise}</p>}
      <footer><button className="secondary" disabled={busy} onClick={() => void generate()}>✨ Tạo bằng AI</button><button className="primary" disabled={!translation.trim() || busy} onClick={() => void saveWord()}>{busy ? 'Đang lưu…' : 'Lưu định nghĩa'}</button></footer>
    </Dialog>}

    {modal === 'review' && <Dialog title="Ôn cùng pet" onClose={() => setModal(null)}>
      {!review ? <div className="empty"><span>✦</span><h2>Hết từ đến hạn</h2><p>Pet nghỉ một chút. Thêm từ mới hoặc quay lại sau.</p></div> : <article className="review-card">
        <small>English → Vietnamese</small>
        {challengeStep === 'preview' || challengeStep === 'complete' ? <div className="term-row">
          <h1>{review.term}</h1>
          <button className="speaker-button" type="button" aria-label={`Nghe phát âm ${review.term}`} onClick={() => speak(review.term)}>🔊</button>
        </div> : <h1 className="hidden-term">?</h1>}
        {review.ipa && <p className="term-ipa">{review.ipa}</p>}
        {challengeStep === 'preview' ? <><p className="meaning">{review.translation}</p>{review.example && <p className="example">“{review.example}”</p>}<button className="primary remember-button" type="button" onClick={() => { setShowMeaning(true); setChallengeStep('letters') }}>Đã nhớ · Bắt đầu thử thách</button></> : showMeaning ? <><p className="meaning">{review.translation}</p>{review.example && <p className="example">“{review.example}”</p>}</> : null}

        {showMeaning && challengeStep === 'letters' && <section className="recall-challenge" aria-label="Thử thách chọn chữ cái">
          <p className="challenge-kicker">Thử thách 1/2 · Ghép từ</p>
          <p className="challenge-prompt">Chọn các ký tự theo đúng thứ tự để ghép từ.</p>
          <div className="chosen-letters" aria-live="polite">{selectedTokenIds.length ? selectedTokenIds.map((id) => letterTokens.find((token) => token.id === id)?.value).join('') : 'Chưa chọn ký tự nào'}</div>
          <div className="letter-choices">{letterTokens.map((token) => <button key={token.id} type="button" className="letter-token" disabled={selectedTokenIds.includes(token.id)} aria-label={`Chọn ký tự ${token.value}`} onClick={() => chooseLetter(token)}>{token.value}</button>)}</div>
        </section>}

        {showMeaning && challengeStep === 'type' && <section className="recall-challenge" aria-label="Thử thách gõ từ">
          <p className="challenge-kicker">Thử thách 2/2 · Tự gõ lại</p>
          <p className="challenge-prompt">Nhập chính xác từ hoặc cụm từ tiếng Anh.</p>
          <form className="challenge-form" onSubmit={(event) => { event.preventDefault(); checkTypedAnswer() }}>
            <input ref={typeAnswerRef} value={typedAnswer} onChange={(event) => { setTypedAnswer(event.target.value); setChallengeFeedback(null) }} autoComplete="off" autoCapitalize="none" autoCorrect="off" spellCheck={false} aria-invalid={Boolean(challengeFeedback)} placeholder="Nhập từ tiếng Anh" />
            <button className="primary" type="submit">Kiểm tra</button>
          </form>
        </section>}

        {showMeaning && challengeFeedback && <p className={`challenge-feedback ${challengeStep === 'complete' ? 'is-correct' : ''}`} role="status">{challengeFeedback}</p>}

        <section className="practice-section" aria-label="Luyện phát âm">
          <button className="secondary toggle-practice" type="button" onClick={() => setPracticeOpen(!practiceOpen)}>
            {practiceOpen ? 'Đóng luyện phát âm' : '🎙 Luyện phát âm (tùy chọn)'}
          </button>
          {practiceOpen && <div className="practice-box">
            <p className="practice-hint">Ghi âm đọc từ rồi bấm Chấm điểm để AI nhận xét. Bạn có thể bỏ qua và rate card bất kỳ lúc nào.</p>
            <div className="practice-controls">
              {!recording && !recordedAudio && (
                <button className="primary small-button" type="button" disabled={busy} onClick={() => void startRecording()}>🎙 Bắt đầu đọc (tối đa 10s)</button>
              )}
              {recording && (
                <button className="secondary danger small-button" type="button" onClick={() => stopRecording()}>⏹ Dừng ghi</button>
              )}
              {recordedAudio && !recording && (
                <>
                  <audio src={recordedAudio} controls className="audio-player" />
                  <button className="secondary small-button" type="button" disabled={busy} onClick={() => void startRecording()}>Ghi lại</button>
                  <button className="secondary small-button" type="button" disabled={busy} onClick={() => clearRecording()}>Bỏ bản ghi</button>
                  <button className="primary small-button" type="button" disabled={busy} onClick={() => void scorePronunciation()}>{busy ? 'Đang chấm…' : '✨ Chấm điểm'}</button>
                </>
              )}
            </div>
            {assessment && (
              <div className="assessment-result">
                <div className="assessment-header">
                  <span className={`score-badge ${assessment.score >= 80 ? 'good' : assessment.score >= 50 ? 'medium' : 'low'}`}>
                    {assessment.score}/100
                  </span>
                  <strong className="assessment-verdict">{assessment.isCorrect ? '✓ Phát âm tốt' : 'Cần cải thiện'}</strong>
                </div>
                {assessment.recognizedText && <p className="recognized"><b>AI nghe:</b> “{assessment.recognizedText}”</p>}
                <p className="feedback">{assessment.feedback}</p>
                {assessment.segments?.length > 0 && (
                  <div className="segments">
                    {assessment.segments.map((seg, idx) => (
                      <span key={idx} className={`segment ${seg.status}`} title={seg.note || undefined}>
                        {seg.part} {seg.status === 'correct' ? '✓' : '✗'}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>}
        </section>

        {challengeStep !== 'complete' && <p className="rating-lock">🔒 Hoàn thành 2 thử thách để chọn mức ôn.</p>}
        <div className="ratings">
          <button className="again" disabled={busy || challengeStep !== 'complete'} onClick={() => void rate(1)}>Again</button>
          <button className="hard" disabled={busy || challengeStep !== 'complete'} onClick={() => void rate(2)}>Hard</button>
          <button className="good" disabled={busy || challengeStep !== 'complete'} onClick={() => void rate(3)}>Good</button>
          <button className="easy" disabled={busy || challengeStep !== 'complete'} onClick={() => void rate(4)}>Easy</button>
        </div>
      </article>}
    </Dialog>}

    {modal === 'inbox' && <Dialog title="Từ vựng đã thêm" onClose={() => setModal(null)} wide className="inbox-dialog">
      <div className="inbox-tools"><input value={search} onChange={(event) => { const value = event.target.value; setSearch(value); void loadVocabulary(value, topicFilter, 0) }} placeholder="Tìm từ hoặc nghĩa" aria-label="Tìm từ hoặc nghĩa" /><select value={topicFilter} onChange={(event) => { const value = event.target.value as TopicFilter; setTopicFilter(value); void loadVocabulary(search, value, 0) }} aria-label="Lọc chủ đề"><option value="">Tất cả chủ đề</option><option value="unclassified">Chưa phân loại</option>{Object.entries(topics).map(([id, label]) => <option value={id} key={id}>{label}</option>)}</select></div>
      <p className="hint">{vocabulary.total} từ · {vocabulary.unclassifiedCount} từ chưa phân loại</p>
      {vocabulary.items.some((word) => !word.translation) && <button className="secondary" disabled={busy} onClick={() => void definePendingWithAi()}>✨ Định nghĩa AI ({vocabulary.items.filter((word) => !word.translation).length} từ đang xem)</button>}
      <div className="inbox-scroll"><div className="vocabulary-list">{vocabulary.items.map((word) => <article className="vocabulary-row" key={word.id}><button className="vocabulary-main" onClick={() => openEdit(word)}><div className="vocabulary-title-line"><strong>{word.term}</strong>{word.ipa && <span className="word-ipa">{word.ipa}</span>}</div><span>{word.translation || 'Chờ định nghĩa'}</span></button><div className="vocabulary-actions"><button className="speaker-button small" type="button" aria-label={`Nghe phát âm ${word.term}`} onClick={(e) => { e.stopPropagation(); speak(word.term) }}>🔊</button>{!word.translation && <><button className="secondary small-button" disabled={busy} onClick={() => void defineWithAi(word)}>✨ AI</button><button className="secondary small-button danger" disabled={busy} onClick={() => void deletePendingWord(word)}>Xóa</button></>}<span className={`topic ${!word.translation ? 'topic-pending' : `topic-${word.topic || 'unclassified'}`}`}>{!word.translation ? 'Chờ định nghĩa' : word.topic ? topics[word.topic] : 'Chưa phân loại'}</span></div></article>)}</div>
      {!vocabulary.items.length && <div className="empty"><span>✦</span><h2>Chưa có từ phù hợp</h2><p>Thêm từ mới hoặc đổi bộ lọc.</p></div>}
      {vocabulary.total > 50 && <footer><button className="secondary" disabled={busy || vocabularyOffset === 0} onClick={() => void loadVocabulary(search, topicFilter, Math.max(0, vocabularyOffset - 50))}>Trước</button><button className="secondary" disabled={busy || vocabularyOffset + 50 >= vocabulary.total} onClick={() => void loadVocabulary(search, topicFilter, vocabularyOffset + 50)}>Sau</button></footer>}</div>
    </Dialog>}

    {modal === 'settings' && <Dialog title="Settings" onClose={() => setModal(null)} wide className="settings-dialog">
      <div className="settings-scroll"><Field label="9Router endpoint"><input value={settings.endpoint} onChange={(event) => setSettings({ ...settings, endpoint: event.target.value })} /></Field>
      <Field label="Model"><div className="inline"><input value={settings.model} list="models" onChange={(event) => setSettings({ ...settings, model: event.target.value })} placeholder="Chọn hoặc nhập model ID" /><button className="secondary" disabled={busy} onClick={() => void loadModels()}>Tải models</button></div><datalist id="models">{models.map((model) => <option value={model} key={model} />)}</datalist></Field>
      <Field label="Model chấm phát âm (hỗ trợ audio)"><div className="inline"><input value={settings.audioModel} list="models" onChange={(event) => setSettings({ ...settings, audioModel: event.target.value })} placeholder="Để trống để dùng model ở trên" /></div><small className="field-hint">Chỉ dùng khi model hỗ trợ input_audio. Để trống sẽ dùng model chung.</small></Field>
      <Field label="9Router API key"><input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={settings.hasApiKey ? 'Đã lưu an toàn. Nhập để thay hoặc xóa.' : 'Tùy chọn, chỉ nếu 9Router yêu cầu'} /></Field>
      <div className="setting-grid"><Field label="Desired retention"><input type="number" min="0.8" max="0.98" step="0.01" value={settings.desiredRetention} onChange={(event) => setSettings({ ...settings, desiredRetention: Number(event.target.value) })} /></Field><Field label="Giờ yên lặng từ"><input type="number" min="0" max="23" value={settings.quietHoursStart} onChange={(event) => setSettings({ ...settings, quietHoursStart: Number(event.target.value) })} /></Field><Field label="đến"><input type="number" min="0" max="23" value={settings.quietHoursEnd} onChange={(event) => setSettings({ ...settings, quietHoursEnd: Number(event.target.value) })} /></Field></div>
      <Field label={`Kích thước pet: ${Math.round(settings.petScale * 100)}%`}><input type="range" min="0.1" max="1.25" step="0.05" value={settings.petScale} onChange={(event) => setSettings({ ...settings, petScale: Number(event.target.value) })} /></Field>
      <Field label={`Độ trong suốt pet: ${Math.round(settings.petOpacity * 100)}%`}><input type="range" min="0.2" max="1" step="0.05" value={settings.petOpacity} onChange={(event) => setSettings({ ...settings, petOpacity: Number(event.target.value) })} /></Field>
      <label className="check"><input type="checkbox" checked={settings.aiEnabled} onChange={(event) => setSettings({ ...settings, aiEnabled: event.target.checked })} /> Cho phép gửi từ đang thêm tới 9Router khi bấm “Tạo bằng AI”</label>
      <label className="check"><input type="checkbox" checked={settings.petPassthrough} onChange={(event) => setSettings({ ...settings, petPassthrough: event.target.checked })} /> Click xuyên pet (mở tray để bật lại tương tác)</label>
      <label className="check"><input type="checkbox" checked={settings.autostart} onChange={(event) => setSettings({ ...settings, autostart: event.target.checked })} /> Mở English Pet khi đăng nhập Windows</label>
      <section className="data-tools" aria-label="Sao lưu dữ liệu">
        <h3>Dữ liệu cục bộ</h3>
        <p>JSON dùng để chuyển dữ liệu. SQLite backup dùng để khôi phục toàn bộ dữ liệu học. Không gồm API key.</p>
        <div className="data-actions"><button className="secondary" disabled={busy} onClick={() => void exportData()}>Export JSON</button><button className="secondary" disabled={busy} onClick={() => void backupSqlite()}>Backup SQLite</button><button className="secondary" disabled={busy} onClick={() => importRef.current?.click()}>Import JSON</button></div>
        <input ref={importRef} className="visually-hidden" type="file" accept="application/json,.json" onChange={(event) => void importData(event)} />
      </section></div>
      <footer><button className="primary" disabled={busy} onClick={() => void saveSettings()}>{busy ? 'Đang lưu…' : 'Lưu Settings'}</button></footer>
    </Dialog>}
  </main>
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label className="field"><span>{label}</span>{children}</label>
}

function ConfirmationDialog({ request, running, onCancel, onConfirm }: { request: ConfirmationRequest; running: boolean; onCancel: () => void; onConfirm: () => void }) {
  return <div className="modal-backdrop confirm-backdrop" role="presentation" onMouseDown={() => { if (!running) onCancel() }}>
    <section className="dialog confirmation-dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirmation-title" aria-describedby="confirmation-description" onMouseDown={(event) => event.stopPropagation()}>
      <header><h2 id="confirmation-title">{request.title}</h2><button disabled={running} onClick={onCancel} aria-label="Đóng">×</button></header>
      <p id="confirmation-description" className="confirmation-copy">{request.description}</p>
      <footer><button className="secondary" autoFocus disabled={running} onClick={onCancel}>Hủy</button><button className={request.danger ? 'primary danger' : 'primary'} disabled={running} onClick={onConfirm}>{running ? 'Đang xử lý…' : request.confirmLabel}</button></footer>
    </section>
  </div>
}

function Dialog({ title, children, onClose, wide = false, className = '' }: { title: string; children: React.ReactNode; onClose: () => void; wide?: boolean; className?: string }) {
  return <div className="modal-backdrop" role="presentation" onMouseDown={onClose}><section className={`dialog ${wide ? 'wide' : ''} ${className}`} role="dialog" aria-modal="true" aria-label={title} onMouseDown={(event) => event.stopPropagation()}><header><h2>{title}</h2><button onClick={onClose} aria-label="Đóng">×</button></header>{children}</section></div>
}

createRoot(document.getElementById('root')!).render(<App />)
