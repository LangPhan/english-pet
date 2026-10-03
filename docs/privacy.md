# Privacy

English Pet stores vocabulary, FSRS state, and review logs in SQLite under Windows app data. No account, cloud database, sync, analytics, or telemetry exists in this MVP.

## Clipboard

English Pet reads clipboard text only after user explicitly presses `Ctrl+Shift+E` or chooses **Thêm từ từ clipboard** in tray. It does not watch clipboard continuously, inspect selected text, or record keystrokes.

## AI and 9Router

AI is optional. Pressing **Tạo bằng AI** sends only current term and optional meaning to configured 9Router endpoint. The default is local `http://localhost:20128/v1`.

Do not use a remote endpoint unless you accept its operator and downstream AI providers may process that content. 9Router controls its own request logging and provider configuration; check its settings before use.

9Router API key, when required, is saved in Windows Credential Manager. It is never written to SQLite, export data, or app logs.

## Backup and restore

In Settings, **Export JSON** writes a versioned vocabulary and FSRS review-history export to `Documents\English Pet Backups`. **Backup SQLite** writes a consistent SQLite snapshot to same folder using SQLite's online backup API. Neither file includes 9Router API key. Release builds ship both per-user NSIS and MSI installers; neither uninstaller removes local learning data or backups automatically.

Import JSON adds only terms not already present, skips duplicate normalized terms, and imports review history only for words added by same import. It never replaces existing cards. Import validates version, size, dates, ratings, and FSRS state before one database transaction commits.

## Delete data

Uninstall removes application files but may leave learning data in Windows app data and files in `Documents\English Pet Backups`. To erase local learning data, quit English Pet then delete its Windows app-data folder and optional backup folder. Delete API key in Settings by clearing field and saving.
