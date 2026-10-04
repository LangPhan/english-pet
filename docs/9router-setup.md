# 9Router setup

1. Install and start 9Router on Windows.
2. Connect providers in 9Router dashboard.
3. In English Pet → Settings, keep endpoint `http://localhost:20128/v1` or enter your own `/v1` endpoint.
4. Click **Tải models**, choose a chat model, then save.
5. If 9Router has API-key auth enabled, paste its local gateway key in **9Router API key** and save. English Pet stores it in Windows Credential Manager.
6. (Optional) For AI pronunciation assessment, configure an audio-capable model in **Model chấm phát âm (hỗ trợ audio)** that supports OpenAI-compatible `input_audio`. If left empty, English Pet uses the primary model.
7. Add/copy an English term. Click **Tạo bằng AI** only when you want an example, IPA pronunciation, and exercise.

## Failures

- Connection error: check 9Router is running and endpoint ends with `/v1`.
- 401: enter/refresh local 9Router API key.
- 503 or model unavailable: use a different model or update accounts/combo in 9Router.
- Invalid JSON: choose a model that follows JSON instructions well, or write meaning/example manually.
- Pronunciation evaluation error: ensure the selected model or gateway provider supports `input_audio` with WAV payloads.

English Pet never configures 9Router providers, OAuth, tunnel, proxy, MITM, cloud sync, or logging. Those stay under user control in 9Router.
