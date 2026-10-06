# English Pet 🐕

Con thú cưng pixel corgi sống trên taskbar Windows, giúp bạn học từ vựng tiếng Anh bằng phương pháp lặp lại cách quãng (FSRS).

![English Pet](src/assets/corgi-run.svg)

## Tính năng

- **Corgi pixel art** — cửa sổ trong suốt, luôn hiện trên màn hình, roam, nhảy, ngủ, ngồi
- **Thu thập từ vựng** — bôi đen từ bất kỳ đâu rồi nhấn `Ctrl+Shift+E`, hoặc dán từ clipboard
- **FSRS spaced repetition** — rating Again/Hard/Good/Easy, tự động lên lịch ôn
- **Gated review** — giải đố sắp xếp chữ cái rồi gõ lại từ trước khi rating
- **AI định nghĩa** — dịch tiếng Việt, IPA, câu ví dụ, bài tập qua OpenAI-compatible endpoint
- **Luyện phát âm** — ghi âm mic, model AI chấm điểm từng âm tiết
- **TTS** — đọc từ offline qua Windows Web Speech API
- **Nhắc ôn tập** — thông báo hệ thống, hỗ trợ snooze, giờ im lặng
- **Tùy chỉnh** — slider kích thước (10–125%), opacity (20–100%), click-through toggle
- **Privacy-first** — không cloud, không telemetry, API key lưu trong Windows Credential Manager

## Tech stack

| | |
|---|---|
| Desktop | Tauri 2 (Rust + WebView) |
| Frontend | React 19 + TypeScript + Vite |
| Database | SQLite (WAL, via rusqlite) |
| Spaced repetition | FSRS (Rust crate) |
| AI | OpenAI-compatible `/v1` (9Router) |

## Cài đặt & Chạy

**Yêu cầu:** Node.js ≥ 20.19, Rust ≥ 1.90, MSVC Build Tools

```bash
npm install

# Dev
npm run tauri dev

# Build installer (NSIS + MSI)
npm run release
```

**Test:**
```bash
# TypeScript type-check
npm run test

# Rust unit tests
cd src-tauri && cargo test
```

## Cấu hình AI

Xem [`docs/9router-setup.md`](docs/9router-setup.md) để kết nối AI endpoint.

## Privacy

Xem [`docs/privacy.md`](docs/privacy.md).

## License

MIT
