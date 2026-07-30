# Zero Gesture Architecture Design Document

> [!NOTE]
> この文書は移行前のWindows実装を説明する。
> 採用済みのマルチプラットフォーム目標設計と移行ゲートは
> [ADR index](./adr/README.md) を正とする。

## 1. Overview

Zero Gesture は、Windows専用の高性能マウスジェスチャーツールです。
**"Hybrid Native/Web Architecture"** を採用し、設定画面の柔軟性と、常駐時の極限までの軽量化・低遅延を両立させます。

### Core Philosophy

- **Zero-Latency Hook:** マウス入力の監視と遮断は、OSのネイティブAPIを直接叩き、最小限のオーバーヘッドで行う。
- **Native Rendering:** ジェスチャー軌跡（Trail）の描画にはWebviewを使用せず、現行実装はGDIで透明なオーバーレイウィンドウに直接描画する。`direct2d.rs`は常に初期化errorを返す未実装stubである。
- **On-Demand Webview:** 設定画面が必要な時のみWebviewをロードし、通常時はメモリから解放する。

---

## 2. System Diagram

```mermaid
graph TD
    subgraph "Main Process (Rust)"
        T[Tray Icon Manager] -->|Spawn| H[Hook Thread]
        T -->|Spawn| O[Overlay Thread]
        T <-->|Invoke/Events| W[Settings Webview]

        SM[State Manager]
        note1[Config / Rules]
        SM -.-> H
    end

    subgraph "Hook Thread (Win32 Message Loop)"
        HM["Mouse Hook (WH_MOUSE_LL)"]
        GL[Gesture Logic]

        HM -->|Raw Coords| GL
        GL -->|Draw Command| O
        GL -->|Action| EX[Executor]
    end

    subgraph "Overlay Thread (Win32 Message Loop)"
        WIN[Transparent Window]

        WIN --> GDI[GDI Renderer]
    end

    subgraph "Frontend (Tauri/Web)"
        UI[React App]
    end

    User((User Input)) --> HM
    EX -->|"SendInput (keyboard only)"| OS((Windows OS))
```

---

## 3. Component Details

### 3.1. Main Thread (Tauri Entrypoint)

アプリケーションのライフサイクルを管理します。

- **Responsibility:**
  - Tauriランタイムの初期化。
  - システムトレイ（タスクトレイ）のアイコンとメニュー管理。
  - Engine Config ownerだけが行う設定ファイルの読み書き（Disk I/O）。
  - 設定画面（Webview）の表示/非表示トグル。
  - immutable compiled configを二つの固定slotへ保持し、generation/indexをatomic publishする。Windows hookからのread wiringはP03cで行う。
  - P03cまでの互換動作として、durable commit/publication後、Applied応答前に既存Hook/Overlay worker pairをcommitted configから再生成する。再生成失敗はdiskをrollbackせずEngine全体を終了し、次のbounded restartでcommitted truthを再読込する。workerはdisk writerではない。
  - Tray labelはworker projection後にTauri main threadへ非同期enqueueする。IPC owner threadは同期menu APIを呼ばず、tray自身の変更はApplied受信後にmain thread上でもlabelを整合する。

### 3.2. Hook Thread (The "Sensor")

UIスレッドのブロックを防ぐため、独立したスレッドでマウス入力を監視します。

- **Technology:** `windows-sys` crate (Win32 API: `SetWindowsHookExW`, `CallNextHookEx`)
- **Responsibility:**
  - **Low-Level Mouse Hook:** `WH_MOUSE_LL` を使用してマウスイベントをフック。
  - **Event Suppression:** ジェスチャー開始トリガー（例: 右クリック）を検知した場合、OSへのイベント伝播をブロック（`1`をreturn）し、コンテキストメニューの出現を防ぐ。
  - **Gesture Recognition:** マウスの移動ベクトルを計算し、定義されたジェスチャー（例: `Right` -> `Down`）と照合する。
  - **Context Resolution:** configured trigger down時、callback内で`WindowFromPoint`、foreground fallback、window情報取得、app matching、target activationを同期実行する。これは移行対象の既知hot-path debtである。目標設計ではcallback外Context workerがpointer sampleからcontext/binding/targetを事前解決し、fresh cacheがないtriggerをpassする意図的な安全変更を行う。
  - **Communication:** 描画commandを`crossbeam-channel`経由で **Overlay Thread** へ送信する。
  - **Action:** callbackでactionをqueueへ積み、同じHook Threadのmessage loopへpostしてcallback復帰後にkeyboard actionだけを実行する。

### 3.3. Overlay Thread (The "Visuals")

TauriのWindow機能を使わず、Rustから直接Win32ウィンドウを作成・制御します。

- **Technology:** `windows-sys` crate (Win32 API, GDI)
- **Responsibility:**
  - **Window Creation:** `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW` スタイルの全画面透明ウィンドウを作成。
  - **Rendering:** Hook Threadから送られてくる座標データを元に、GDI（`Polyline` + バックバッファビットマップ）を用いてラインを描画する。移行でもGDIを維持し、別rendererの採用は性能契約の未達を測定した後に別ADRで判断する。Direct2Dは未実装（常にerrorを返すstubのみ）。
  - **Lifecycle:** ジェスチャー中のみ可視化（`ShowWindow`）し、終了後は非表示＆描画クリアを行うことでリソースを節約。

### 3.4. Settings UI (The "Interface")

ユーザーがジェスチャー定義を編集するための画面です。

- **Technology:** Tauri Frontend (React 19 + Tailwind CSS v4 + react-aria-components + TanStack Router)
- **Responsibility:**
  - ジェスチャーとアクションのマッピング編集。
  - 軌跡の色・太さの設定。
  - Tauri Commandを経由してRust側の設定ファイルを更新。
  - edit/import開始時にEngineから観測したrevisionを保持し、Prepareへ渡す。Applied後は返されたconfig/revisionでquery cacheを置換し、Windows metadata durability warningを表示する。
  - **Performance Note:** この画面が開いていない時、Webviewプロセスは存在しないか、サスペンド状態になるように管理する。

---

## 4. Data Flow

### Scenario: User performs "Right-Drag Down" (Minimize Window)

1.  **Trigger:** ユーザーが右ボタンを押下 (`WM_RBUTTONDOWN`)。
2.  **Intercept:** **Hook Thread** がイベントを検知。設定に基づき、イベントをOSに渡さずに握りつぶす。
3.  **Start:** ジェスチャーモードに移行。**Overlay Thread** に「表示開始」シグナルを送信。
4.  **Tracking:** ユーザーがマウスを下に移動。
    - **Hook Thread** は座標をサンプリングし、ジェスチャー方向「Down」を判定。
    - 同時に座標を **Overlay Thread** へ送信。
5.  **Rendering:** **Overlay Thread** が受信した座標を元に、画面上に線をリアルタイム描画。
6.  **Release:** ユーザーが右ボタンを離す (`WM_RBUTTONUP`)。
7.  **Execution:**
    - **Hook Thread** がジェスチャー完了を検知。
    - 「Down」に対応するアクション（`Win+Down` キー送信など）を実行。
    - **Overlay Thread** に「終了」シグナルを送信。
8.  **Cleanup:** オーバーレイが消去され、ウィンドウが非表示になる。

---

## 5. Technology Stack & Crates

| Category             | Technology / Crate                 | Purpose                                          |
| :------------------- | :--------------------------------- | :----------------------------------------------- |
| **App Framework**    | `tauri` v2                         | アプリケーションシェル、設定UI、ビルドシステム   |
| **Windows API**      | `windows-sys`                      | Win32 APIへのRawアクセス (Hooks, GDI, Input)     |
| **Concurrency**      | `std::thread`, `crossbeam-channel` | スレッド管理と高速なメッセージパッシング         |
| **State Mngt**       | Engine owner + fixed two-slot publication | 設定mutationの単一所有とlock-free snapshot read |
| **Serialization**    | `serde`, `serde_json`              | 設定ファイルの保存・読み込み                     |
| **Logging**          | `log`, `tauri-plugin-log`          | ログ出力                                         |
| **Input Simulation** | `windows-sys` (SendInput)          | キーボード・マウス操作の自動実行                 |
| **Pattern Matching** | `regex`                            | アプリ名マッチング                               |
| **Frontend**         | React 19 + TypeScript + TanStack Router + react-aria-components | 設定画面のUI構築 |

---

## 6. Directory Structure

```text
/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs             // Entry point, Tauri setup
│   │   ├── lib.rs              // WorkerThreads管理、スレッド起動
│   │   ├── config/             // schema v2、legacy migration、immutable compile
│   │   ├── commands.rs         // Tauri IPC コマンドハンドラ
│   │   ├── executor.rs         // アクション実行 (SendInput等)
│   │   ├── domain/
│   │   │   ├── mod.rs         // portable gesture module interface
│   │   │   ├── recognition.rs // ジェスチャー方向計算
│   │   │   └── session.rs     // session stateとclosed decision/effect
│   │   ├── tray.rs             // システムトレイ管理
│   │   ├── capture.rs          // ウィンドウキャプチャ
│   │   ├── window_info.rs      // アクティブウィンドウ情報取得
│   │   ├── log.rs              // ログ設定
│   │   ├── hook/
│   │   │   ├── mod.rs          // フック起動・バインディングコンパイル
│   │   │   ├── app_match.rs    // アプリ名マッチング
│   │   │   └── win32.rs        // Win32 callback、変換、effect適用
│   │   └── overlay/
│   │       ├── mod.rs          // TrailRendererトレイト、OverlayCommand定義
│   │       ├── window.rs       // Win32ウィンドウ作成・メッセージループ
│   │       ├── gdi.rs          // GDIレンダラー実装
│   │       └── direct2d.rs     // Direct2Dレンダラー（未実装スタブ）
│   ├── Cargo.toml
│   └── build.rs
├── src/                        // Frontend (Settings UI)
│   ├── main.tsx
│   ├── routes/                 // TanStack Router ファイルベースルーティング
│   └── components/
└── docs/
    └── architecture.md
```

## 7. Performance Considerations

- **Blocking:** 目標設計ではHook callbackをnormalize/evaluate、essential creditのnonblocking reserve/send、best-effort render send、pass/suppress returnの順に限定する。現行callbackはconfigured trigger down時のwindow activation/query/app matching、overlay channel送信、action/replay queueingを同期実行しており、ADR 0002に従って移行する必要がある。
- **Memory Safety:** `unsafe` ブロックを多用するWin32 API部分は、Rustのラッパー関数で適切に抽象化し、メモリリークや未定義動作を防ぐ。
- **Drawing:** 現在とWindows移行中はGDI（`Polyline` + バックバッファ）を使用する。`direct2d.rs`は未実装stubであり、性能契約の未達を測定した場合だけ別ADR/PRでrenderer変更を検討する。macOSのAppKit/Core Animation adapterはこのWindows判断と分離する。
