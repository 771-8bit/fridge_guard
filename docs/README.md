# Fridge Guard ドキュメント一覧 (Documentation Index)

本ディレクトリ（`docs/`）には、Fridge Guard の設計、導入、開発、運用、セキュリティに関する詳細ドキュメントが格納されています。

目的に応じて以下の各ガイドを参照してください。


---

## 1. はじめての方・導入ガイド (Getting Started)

デバイスを組み立て、LINE通知を受け取るまでの手順書です。

| ドキュメント | 概要 | 対象 |
|---|---|---|
| **[INSTALLATION.md](INSTALLATION.md)** | SPS-320NセンサとGroveケーブルの加工・結線、NanoC6への接続、冷蔵庫本体への設置方法 | 全員 |
| **[LINE_SETUP.md](LINE_SETUP.md)** | LINE Developers Console での Messaging API チャネル開設、トークン取得、User ID キャプチャツールの使い方 | 全員 |
| **[PROVISIONING.md](PROVISIONING.md)** | 付属ツール `tools/provision.py` を用いた、USBシリアル経由でのWi-Fi・LINE設定の登録手順 | 全員 |

---

## 2. 開発・ビルド環境 (Development & Build)

ファームウェアのソースコード変更やテスト、ローカルコンパイルを行うためのガイドです。

| ドキュメント | 概要 | 対象 |
|---|---|---|
| **[RUST_TOOLCHAIN.md](RUST_TOOLCHAIN.md)** | ESP32-C6 (RISC-V) 向けクロスコンパイル環境（`espflash`, `ldproxy`, nightlyツールチェイン）のセットアップ | 開発者 |
| **[BUILD.md](BUILD.md)** | ホスト側単体テスト実行（`cargo test`）、ファームウェアビルド、実機フラッシュ書き込み手順 | 開発者 |
| **[VERIFICATION.md](VERIFICATION.md)** | デバウンス、状態遷移、月間クォータ、リカバリ等の受入テスト（Acceptance Test）仕様とエビデンス | 開発者 |

---

## 3. 運用・リリース・セキュリティ (Operations & Security)

安全な運用、OTAアップデート、より高度なハードウェア暗号化に関するガイドです。

| ドキュメント | 概要 | 対象 |
|---|---|---|
| **[OTA_RELEASE.md](OTA_RELEASE.md)** | GitHub Releases と連携した安全な OTA ファームウェア配信、マニフェスト生成（`tools/release.py`）手順 | メンテナー |
| **[SECURITY_OPERATIONS.md](SECURITY_OPERATIONS.md)** | 認証情報の保護、CIによるシークレットスキャン、安全な運用オペレーション | メンテナー |
| **[OPTIONAL_DEVICE_HARDENING.md](OPTIONAL_DEVICE_HARDENING.md)** | ESP32-C6 のハードウェア暗号化（Flash Encryption）および署名検証（Secure Boot v2）の適用手順 | 上級者 |
| **[TROUBLESHOOTING.md](TROUBLESHOOTING.md)** | Wi-Fi接続失敗、LINE通知不達、センサ誤検知、OTAエラー等の原因別対処マトリクス | 全員 |
| **[implementation_decisions.md](implementation_decisions.md)** | 完全サーバレス方式の選定、M5Stack NanoC6採用、Rust on ESP-IDF選定等の設計意思決定記録 | 全員 |
