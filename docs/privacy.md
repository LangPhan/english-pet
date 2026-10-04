# Privacy

English Pet stores vocabulary, FSRS state, and review logs in SQLite under Windows app data. No account, cloud database, sync, analytics, or telemetry exists in this MVP.

## Clipboard

When user explicitly presses `Ctrl+Shift+E`, English Pet waits for shortcut keys to release, sends one Windows copy command to focused highlighted text, then reads only resulting text locally. Copied selection remains on clipboard, same as a normal copy. Clicking **+ Từ** or choosing **Thêm từ từ clipboard** in tray reads existing clipboard text instead. It animates text on a bone into Corgi, then saves it locally in Inbox as **Chờ định nghĩa**. It does not continuously watch clipboard or selections, record keystrokes, use UI Automation, or use OCR.

## AI and 9Router

AI is optional. Pressing **Tạo bằng AI** for one Inbox word sends only that term and optional meaning to configured 9Router endpoint, then returns meaning, example, exercise, and one fixed topic. The default is local `http://localhost:20128/v1`.

Viewing, searching, and filtering the local vocabulary Inbox never sends data over the network. **Định nghĩa AI** only sends the explicitly confirmed pending terms currently shown in Inbox; it returns meaning, example, and fixed topic. It never runs automatically or sends review logs, statistics, or the full database. Pending words do not enter review reminders until they have a Vietnamese meaning.

Do not use a remote endpoint unless you accept its operator and downstream AI providers may process that content. 9Router controls its own request logging and provider configuration; check its settings before use.

9Router API key, when required, is saved in Windows Credential Manager. It is never written to SQLite, export data, or app logs.

## Audio and Pronunciation

- **Speaker playback (TTS)**: Clicking 🔊 plays native pronunciation using Windows Web Speech API (`SpeechSynthesis`). It runs completely offline on device, creates no network traffic, and downloads or saves no audio files.
- **Microphone and pronunciation assessment**: Microphone access is only requested when you explicitly click **Bắt đầu đọc** in the review dialog's optional practice section. Audio is captured in-memory only, capped at 10 seconds.
- Voice recordings and assessment scores are **never** saved to SQLite, local backup files, exports, or app logs. Discarding, re-recording, closing the dialog, or moving to the next card immediately releases recorded audio.
- Audio and the target word/IPA leave your computer **only** when you explicitly click **Chấm điểm**, sent directly to your configured 9Router endpoint. If you do not wish to evaluate pronunciation or lack microphone access, you can skip pronunciation practice and rate your review card directly at any time.

## Backup and restore

In Settings, **Export JSON** writes a version 2 vocabulary, optional topic, and FSRS review-history export to `Documents\English Pet Backups`; English Pet still imports version 1 exports. **Backup SQLite** writes a consistent SQLite snapshot to same folder using SQLite's online backup API. Neither file includes 9Router API key. Release builds ship both per-user NSIS and MSI installers; neither uninstaller removes local learning data or backups automatically.

Import JSON adds only terms not already present, skips duplicate normalized terms, and imports review history only for words added by same import. It never replaces existing cards. Import validates version, size, dates, ratings, and FSRS state before one database transaction commits.

## Delete data

Inbox can delete only words still marked **Chờ định nghĩa**; defined or reviewed cards cannot be deleted there. This deletion stays local and sends no data over network.

Uninstall removes application files but may leave learning data in Windows app data and files in `Documents\English Pet Backups`. To erase local learning data, quit English Pet then delete its Windows app-data folder and optional backup folder. Delete API key in Settings by clearing field and saving.
