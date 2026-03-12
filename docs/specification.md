# fterm 関数仕様書 (Fish → Rust 移行用)

## 概要

fterm は Fish shell で書かれた SSH/SCP 接続管理ツール。fuzzy finder による対話的ホスト選択、12 項目の SSH config バリデーション、tmux 連携のセッションログ記録を提供する。

### アーキテクチャ方針

- **tmux は維持する**: ペイン分割 (水平/垂直)、ペインリサイズ、ペインボーダーへの `ssh:<hostname>` 表示に使用
- **ログ記録の改善**: `pipe-pane` で呼び出すフィルタを Rust 製 (`fterm log-filter`) に置き換え、`ansifilter` + `awk` 依存を排除

---

## 1. ユーザ向けコマンド

### 1.1 `fssh` — 対話的 SSH ホスト選択

**ファイル:** `functions/fssh.fish`

**仕様:**

- 引数: なし
- 戻り値: なし (commandline に `ssh <hostname>` を設定)
- 前処理: `FTERM_LOG_DIR_PREFIX` 下の未圧縮 `.log` ファイルを `gzip` で圧縮
- `__fssh_get_hosts` でホスト一覧を取得
- `fzf` で対話的に選択 (preview: `rg` で SSH config の該当ブロック表示)
- 選択されたホスト名を `commandline` に挿入 (実行はしない)
- `FZF_DEFAULT_OPTS` が未設定の場合のみデフォルト値を設定し、終了時に復元

**依存:** `find`, `gzip`, `fzf`, `rg`, `__fssh_get_hosts`

---

### 1.2 `ssh` — SSH ラッパー (バリデーション + ログ + tmux)

**ファイル:** `functions/ssh.fish`

**仕様:**

- 引数: 標準の `ssh` コマンド引数すべて
- 戻り値: 関数内で SSH の終了ステータスを明示的にキャプチャしていないため、最後に実行されたコマンド (`__fterm_stop_logging` またはキャッシュクリア) のステータスが暗黙的に返る
- ターゲットホスト: `$argv[-1]` (最後の引数)

**処理フロー:**

1. 開始時刻を epoch 秒で記録
2. dry-run チェック (`-G`, `-V`, `-Q`, `--help`): dry-run なら tmux/バリデーション/ログをスキップ
3. tmux 内で実行されていなければ `__fterm_ensure_tmux` で tmux セッションを作成・接続 → return 1 で終了 (tmux 内で再実行される)
4. `SSH_ENV` ファイルが存在すれば `source` で読み込み
5. `ssh-add -l` を 1 秒タイムアウトで実行 → 失敗時は **一律** return 1 で中断 (`-i` 指定の有無は考慮しない)
6. 結果をグローバル変数 `__fterm_cached_agent_keys` / `__fterm_cached_agent_status` にキャッシュ
7. `__fssh_config_check "$target_host"` で 12 項目バリデーション → エラー時は return 1
8. `__fssh_ssh_get_connection_info` で user/host/port を取得
9. ログファイルパス生成: `{FTERM_LOG_DIR_PREFIX}{YYYY/MM/DD}/{YYYYMMDDTHHMMSS}_{session}-{window}{pane}_ssh_{user}@{host}.log`
   - `target_host` に `@` が含まれる場合はそのまま使用
10. `__fterm_start_logging` でログ記録開始
11. `__fssh_ssh_status_splash` で接続情報バナー表示
12. Windows MSYS2 の場合 HOME を `cygpath -m` に変更 (Include 解決のため)
13. tmux ペインタイトルを `ssh:{user}@{target_host}` に設定
14. ウィンドウの SSH 接続カウンタ `@fterm_ssh_count` をインクリメント
15. `automatic-rename` と `allow-rename` を off に設定
16. `command "$ssh_cmd" $config_args $argv` で SSH 実行
17. HOME を復元
18. 接続時間を計算し切断バナー表示
19. `tmux select-pane -P 'default'` でペインスタイルをリセット
20. ペインタイトルを復元
21. `@fterm_ssh_host` ペインオプションを削除
22. `@fterm_ssh_count` をデクリメント → 0 になったら `allow-rename` と `automatic-rename` を on に復元
23. `__fterm_stop_logging` でログ停止 + gzip 圧縮
24. エージェントキーキャッシュをクリア

**グローバル変数 (副作用):**

- `__fterm_cached_agent_keys` (一時的、関数終了時にクリア)
- `__fterm_cached_agent_status` (一時的、関数終了時にクリア)
- tmux ウィンドウオプション: `@fterm_ssh_count`
- tmux ペインオプション: `@fterm_ssh_host`

---

### 1.3 `scp` — SCP ラッパー

**ファイル:** `functions/scp.fish`

**仕様:**

- 引数: 標準の `scp` コマンド引数すべて
- 戻り値: SCP コマンドの終了ステータス
- `ssh.fish` とほぼ同じフローだが以下が異なる:
  - `__fssh_scp_extract_hosts` で引数からリモートホストを抽出 (複数ホスト対応)
  - 各ホストに対して個別にバリデーション
  - ログファイル名: `..._scp_{hosts}.log` (ホスト名リストをアンダースコア結合)
  - 転送結果 (成功/失敗) をカラー付きで表示
  - `@fterm_ssh_count` カウンタは使用しない (切断時に常に rename 復元)

---

### 1.4 `flog` — ログビューア

**ファイル:** `functions/flog.fish`

**仕様:**

- 引数: なし
- `FTERM_LOG_DIR_PREFIX` が未設定/存在しない場合は return 1
- 2 つのモードを持つ fzf ベースのビューア:
  - **File モード** (デフォルト): ファイル名で検索、新しい順にソート
  - **Search モード** (Ctrl-S): `rg --search-zip` でファイル内容を全文検索
- Ctrl-F で File モードに戻る
- プレビュー: `.gz` なら `zcat | head --lines=500`、通常なら `head --lines=500`
- 選択後: `.gz` なら `zcat '<path>' | less`、通常なら `less '<path>'` を commandline に設定

**注意:** `--preview` 引数内のコードは fzf が bash/sh で実行するため、bash 構文 (`if [[ ... ]]`) を使用している (Fish 構文ではない)

---

### 1.5 `fgen` — SSH config テンプレート生成

**ファイル:** `functions/fgen.fish`

**仕様:**

- 引数: なし (対話式)
- テンプレートファイル: `$HOME/.ssh/template.conf`
  - 存在しない場合: デフォルトテンプレートを生成して return 0
- 対話プロンプト:
  - organization 名の入力 (空入力はループで再要求)
  - environment 名の入力 (空入力はループで再要求)
- 出力先: `$ssh_home/conf.d/envs/{org}/{env}.conf`
  - 既存ファイルがある場合は上書き確認 (y/Y で続行)
- sed で 4 つの置換を実行:
  1. `org.dev` → `{org}.{env}`
  2. `org.env` → `{org}.{env}`
  3. `org_dev` → `{org}_{env}`
  4. `org_env` → `{org}_{env}`
- 生成結果の先頭 20 行をプレビュー表示

---

## 2. SSH Config パース関数

### 2.1 `__fssh_get_ssh_home` — SSH ホームディレクトリ解決

**ファイル:** `functions/__fssh_get_ssh_home.fish`

**仕様:**

- 引数: なし
- 出力: SSH ホームディレクトリのパス
- ロジック:
  - MSYS2 (`MSYSTEM` かつ `USERPROFILE` が設定済み): `$(cygpath -u "$USERPROFILE")/.ssh`
  - その他: `$HOME/.ssh`

---

### 2.2 `__fssh_get_included_files` — Include ディレクティブの再帰解決

**ファイル:** `functions/__fssh_get_included_files.fish`

**仕様:**

- 引数:
  - `argv[1]`: config ファイルパス (省略時: `$ssh_home/config`)
  - `argv[2]`: SSH ホームディレクトリ (省略時: `__fssh_get_ssh_home` の結果)
- 出力: 全 Include チェーン内のファイルパス (1 行 1 パス、走査順)
- 戻り値: 0 (成功)、ファイル未存在時は暗黙の return

**アルゴリズム:**

1. グローバル変数 `__fssh_visited_files` で訪問済みファイルを追跡 (循環参照防止)
2. 現在のファイルを出力
3. 各行を読み取り:
   - コメント行 (`^\s*#`) と空行をスキップ
   - `Include` ディレクティブ (大文字小文字無視) を検出
   - パターンを空白分割して個別処理
   - パス解決: `~` → HOME 展開、`/` → 絶対パス、その他 → `$ssh_home/` 相対
   - グロブ展開: `builtin eval "set expanded_files $resolved_pattern"` で実行
   - 展開されたファイルに対して再帰呼び出し

**副作用:** グローバル変数 `__fssh_visited_files` を作成・変更 (呼び出し元の `__fssh_get_hosts` / `__fssh_check_host_prefix` で erase)

**注意点:** line 91 の `eval` によるグロブ展開は、SSH config が信頼できるソースであることを前提としている

---

### 2.3 `__fssh_get_hosts` — ホスト一覧取得

**ファイル:** `functions/__fssh_get_hosts.fish`

**仕様:**

- 引数: なし
- 出力: ソート済みユニークなホスト名リスト (1 行 1 ホスト)
- 戻り値: 0 (成功)、config 未存在時は return

**アルゴリズム:**

1. `__fssh_get_included_files` で全 config ファイルを取得
2. 各ファイルの `Host` で始まる行からホスト名を抽出
3. ワイルドカード (`*` または `?` を含む) をフィルタ
4. `sort --unique` でソート・重複排除

**副作用:** `__fssh_visited_files` を erase (呼び出し前後で)

---

### 2.4 `__fssh_ssh_get_connection_info` — 接続情報取得

**ファイル:** `functions/__fssh_ssh_get_connection_info.fish`

**仕様:**

- 引数: `argv[1]` — ターゲットホスト名
- 出力: `user\thostname\tport` (タブ区切り)。3 フィールドすべて揃わない場合は空出力
- MSYS2 では HOME を `cygpath -m` に一時変更して `ssh -G` を実行

---

### 2.5 `__fterm_get_ssh_config_args` — SSH config 引数生成

**ファイル:** `functions/__fterm_get_ssh_config_args.fish`

**仕様:**

- 引数: なし
- 出力: `__fssh_ssh_config` が設定済みなら `-F` と config パスを出力 (2 要素)。未設定なら空

---

### 2.6 `__fterm_get_ssh_config_details` — SSH 設定詳細取得 (ログ用)

**ファイル:** `functions/__fterm_get_ssh_config_details.fish`

**仕様:**

- 引数: `argv[1]` — ターゲットホスト名
- 出力: デフォルト以外の SSH 設定値 (1 行 1 項目):
  - ProxyJump (none 以外)
  - ProxyCommand (none 以外)
  - IdentityFile (すべて)
  - IdentitiesOnly (すべて)
  - ForwardAgent (yes のみ)
  - LocalForward (すべて)
  - RemoteForward (すべて)
  - DynamicForward (none 以外)

---

### 2.7 `__fterm_get_matched_agent_keys` — エージェント鍵照合

**ファイル:** `functions/__fterm_get_matched_agent_keys.fish`

**仕様:**

- 引数: `argv[1]` — ターゲットホスト名
- 出力: マッチした鍵の情報 (`<agent_key_line> (from: <identity_file>)`)
- `ssh -G` で IdentityFile 一覧を取得
- `ssh-add -l` でエージェント鍵を取得 (キャッシュがあれば使用)
- 各 IdentityFile の `ssh-keygen -lf` フィンガープリントとエージェント鍵を照合

---

### 2.8 `__fterm_get_ssh_cmd` — SSH コマンドパス

**ファイル:** `functions/__fterm_get_ssh_cmd.fish`

**仕様:**

- 引数: なし
- 出力: `__fssh_ssh_cmd` が設定済みならその値、未設定なら `ssh`

---

### 2.9 `__fssh_get_scp_cmd` — SCP コマンドパス

**ファイル:** `functions/__fssh_get_scp_cmd.fish`

**仕様:**

- 引数: なし
- 出力: `__fssh_scp_cmd` が設定済みならその値、未設定なら `scp`

---

### 2.10 `__fterm_run_ssh_cmd` — SSH コマンド実行 (タイムアウト付き)

**ファイル:** `functions/__fterm_run_ssh_cmd.fish`

**仕様:**

- 引数: `argv[1]` — コマンド名 (`ssh`, `ssh-add`, `ssh-keygen`)、`argv[2..-1]` — コマンド引数
- 出力: コマンドの stdout (CR 除去済み)
- 戻り値: コマンドの終了ステータス (`$pipestatus[1]` で timeout コマンドのステータスを取得)

**動作:**

1. コマンド名でパスを解決 (`__fssh_ssh_cmd` 等のグローバル変数優先)
2. `timeout --foreground --kill-after=1 1` で 1 秒タイムアウト実行
3. stderr は `/dev/null` にリダイレクト
4. `string replace -a \r ''` で CR を除去 (Windows 互換)
5. 非 0 ステータスの場合は出力せずにステータスのみ return

**設計意図:** gpg-agent がフリーズした場合にターミナルを固まらせないための保護。`ssh -G`, `ssh-add -l`, `ssh-keygen -lf` 等の高速コマンド専用。

---

## 3. SSH Config バリデーション関数

### 3.1 `__fssh_config_check` — バリデーションオーケストレータ

**ファイル:** `functions/__fssh_config_check.fish`

**仕様:**

- 引数: `argv[1]` — ターゲットホスト (省略時は全ホストをチェック)
- 戻り値: 0 (エラーなし)、1 (エラーあり)
- グローバル変数 `__fssh_check_messages` にメッセージを蓄積
  - メッセージ形式: `{level}:{text}` (level: `E` = error, `W` = warn)

**チェック実行順序:**

1. `__fssh_check_cm_dir` — ControlMaster ディレクトリ
2. `__fssh_check_syntax` — 構文チェック (失敗時は即座に return 1)
3. `__fssh_check_hosts_duplicate` — 重複ホスト検出
4. 各ホストに対して:
   - `__fssh_check_host_prefix` — 命名規則
   - `__fssh_check_basic_config` — 必須/推奨フィールド
   - `__fssh_check_identity_file` — 鍵ファイル検証
   - `__fssh_check_proxyjump` — ProxyJump 検証
   - `__fssh_check_control_path` — ControlPath 検証

**エラー計数方式:** 各チェック関数の stdout から `^ERROR:` / `^WARN:` 行を grep でカウント

**サマリー表示:**

- エラーあり: 赤色で `[ERROR] Config check failed: N error(s), M warning(s)`
- 警告のみ: 黄色で `[WARN ] Config check passed with M warning(s)`
- 問題なし: 緑色で `[OK   ] Config check passed`
- 蓄積されたメッセージを色付きで表示後、`__fssh_check_messages` を erase

---

### 3.2 `__fssh_check_syntax` — 構文チェック

**ファイル:** `functions/__fssh_check_syntax.fish`

**仕様:**

- 引数: なし
- 戻り値: 0 (構文 OK)、1 (構文エラー)
- `ssh -G syntax.check.dummy.host` でダミーホストに対して全 config をパース
- 失敗時: 出力から `(error|bad|unknown|invalid)` にマッチする行を表示

---

### 3.3 `__fssh_check_basic_config` — 必須/推奨フィールドチェック

**ファイル:** `functions/__fssh_check_basic_config.fish`

**仕様:**

- 引数: `argv[1]` — ホスト名
- 戻り値: 1 (エラーあり)、0 (それ以外)
- stdout: `ERROR:*` / `WARN:*` 行

**チェック項目:**

| フィールド     | レベル | 条件                                                                   |
| -------------- | ------ | ---------------------------------------------------------------------- |
| HostName       | ERROR  | 空 または ホスト名と同一 かつ ホスト名が `*.*.*` パターン (3 部分以上) |
| User           | ERROR  | 空                                                                     |
| Port           | ERROR  | 空                                                                     |
| IdentitiesOnly | WARN   | `yes` 以外                                                             |
| IdentityFile   | WARN   | 未設定                                                                 |

**HostName チェックの意図:** `ssh -G` は HostName 未設定時にエイリアスをそのまま返す。`org.env.hostname` のような 3 部分以上のエイリアスで HostName が設定されていない場合はエラー扱い。2 部分以下 (例: `bastion`) は実際のホスト名の可能性があるため許容。

---

### 3.4 `__fssh_check_identity_file` — 鍵ファイル検証

**ファイル:** `functions/__fssh_check_identity_file.fish`

**仕様:**

- 引数: `argv[1]` — ホスト名
- 戻り値: 1 (エラーあり)、0 (それ以外)
- IdentityFile 未設定の場合は return 0

**各 IdentityFile に対するチェック:**

1. `~` を HOME に展開、MSYS2 では `cygpath -u` でテスト用パスに変換
2. ファイル存在確認 → ERROR if not found
3. 鍵タイプ判定:
   - `ssh-keygen -y -f <file>` 成功 → 秘密鍵
   - 失敗後 `ssh-keygen -lf <file>` 成功 → 公開鍵
   - 両方失敗 → ERROR (無効な鍵)
4. 公開鍵の場合:
   - エージェント利用可能: フィンガープリントがエージェントに存在しなければ ERROR
   - エージェント利用不可: ERROR
5. 秘密鍵の場合:
   - WARN (直接指定は非推奨)
   - MSYS2 以外: `stat -c '%a'` でパーミッション確認、`600` 以外なら WARN

---

### 3.5 `__fssh_check_proxyjump` — ProxyJump 検証

**ファイル:** `functions/__fssh_check_proxyjump.fish`

**仕様:**

- 引数: `argv[1]` — ホスト名、`argv[2..-1]` — 訪問済みホストリスト (再帰用)
- 戻り値: 循環参照検出時・proxyjump 未発見時は 1、**それ以外は常に 0** (proxy_host の config/identity エラー検出時も 0 を返す — バグ)
- `ssh -G` で ProxyJump 値を取得
- ProxyJump が未設定または `none` なら return 0
- カンマ区切りの ProxyJump チェーン対応

**各 proxy_host に対するチェック:**

1. 循環参照チェック: `visited_hosts` に含まれていれば ERROR + return 1
2. config 内のホスト一覧に存在しない場合:
   - `user@host` 形式 → 許容 (直接接続)
   - IPv4 アドレス → 許容
   - ドットなしのシンプルなホスト名 → 許容
   - それ以外 → ERROR + return 1
3. config 内に存在する場合:
   - `__fssh_check_basic_config` で基本設定を再帰チェック
   - `__fssh_check_identity_file` で鍵を再帰チェック
   - `__fssh_check_proxyjump` で ProxyJump チェーンを再帰チェック

**既知のバグ (Rust 版で修正):** 現行の Fish 実装は proxy_host の config/identity エラー発見時も `return 0` する (line 116)。エラーは stdout に echo され orchestrator の grep でカウントされるため実用上は動作するが、戻り値が不正確。Rust 版ではエラー検出時に適切なステータスを返す。

---

### 3.6 `__fssh_check_control_path` — ControlPath 検証

**ファイル:** `functions/__fssh_check_control_path.fish`

**仕様:**

- 引数: `argv[1]` — ホスト名
- 戻り値: 常に 0
- `ssh -G` で ControlMaster と ControlPath を取得
- ControlMaster が `no` または空、ControlPath が `none` または空ならスキップ
- `dirname` でディレクトリ部分を抽出 (プレースホルダ `%C` `%h` 等はそのまま)
- `~` を HOME に展開
- ディレクトリが存在しなければ WARN、書き込み不可なら WARN

---

### 3.7 `__fssh_check_cm_dir` — ControlMaster ディレクトリ作成

**ファイル:** `functions/__fssh_check_cm_dir.fish`

**仕様:**

- 引数: なし
- 出力: `created` (作成した)、`error` (作成失敗)、空 (既に存在)
- 戻り値: 0 (成功)、1 (作成失敗)
- 対象: `$ssh_home/conf.d/cm`
- 存在しなければ `mkdir -p` + `chmod 700` で作成

---

### 3.8 `__fssh_check_host_prefix` — ホスト命名規則チェック

**ファイル:** `functions/__fssh_check_host_prefix.fish`

**仕様:**

- 引数: `argv[1]` — ホスト名
- 戻り値: 0 (OK)、1 (ワイルドカードパターン未発見)
- ホスト名をドットで分割、パーツ数 < 2 ならスキップ
- 最後のパーツ以外でプレフィックスを構築 (例: `org.env.host` → `org.env`)
- config 内で `Host {prefix}.*` パターンを検索
- 未発見の場合: より広いパターン (`org.*`) や `Host *` をフォールバック検索
- すべて未発見なら ERROR

---

### 3.9 `__fssh_check_hosts_duplicate` — 重複ホスト検出

**ファイル:** `functions/__fssh_check_hosts_duplicate.fish`

**仕様:**

- 引数: なし
- 出力: 重複ホスト名 (1 行 1 ホスト)
- 戻り値: 常に 0
- 全 config ファイルから Host エントリとソースファイルを収集
- ワイルドカード含むホストはスキップ
- O(n²) アルゴリズムで重複を検出、初回と再発見の位置を報告

---

### 3.10 `__fssh_get_config_value` — 個別設定値取得ヘルパー

**ファイル:** `functions/__fssh_config_check.fish` 内に定義

**仕様:**

- 引数: `argv[1]` — ホスト名、`argv[2]` — 設定キー (小文字)
- 出力: 設定値
- `ssh -G <host>` の出力から awk で指定キーの値を抽出

---

## 4. ログ・tmux 連携関数

### 4.1 `__fterm_ensure_tmux` — tmux セッション確保

**ファイル:** `functions/__fterm_ensure_tmux.fish`

**仕様:**

- 引数: `argv[1]` — コマンド名、`argv[2..-1]` — コマンド引数
- 戻り値: 0 (既に tmux 内)、1 (tmux に委譲した)
- tmux 未インストールなら return 1
- `$TMUX` 設定済みなら return 0 (既に tmux 内)
- 引数を `string escape` でエスケープしてコマンド文字列を構築
- `login-session` が存在すれば attach、なければ new-session
- `send-keys` でコマンドを tmux 内に送信
- return 1 → 呼び出し元は終了 (コマンドは tmux 内で再実行される)

---

### 4.2 `__fterm_start_logging` — ログ記録開始

**ファイル:** `functions/__fterm_start_logging.fish`

**仕様:**

- 引数: `argv[1]` — ログファイルパス、`argv[2]` — ターゲットホスト、`argv[3]` — SSH 情報
- ログディレクトリを `mkdir -p` で作成
- ログファイルのヘッダーに SSH 設定詳細とエージェント鍵情報を書き込み
- `tmux pipe-pane` でペイン出力をパイプ:
  ```
  exec cat - | ansifilter | awk '{print strftime("[%Y-%m-%dT%H:%M:%S%z]"), $0; fflush()}' >> {logfile}
  ```
- ペイン ID ベースの tmux オプション `@{pane_id}` に `logging` を設定

---

### 4.3 `__fterm_stop_logging` — ログ記録停止

**ファイル:** `functions/__fterm_stop_logging.fish`

**仕様:**

- 引数: `argv[1]` — ログファイルパス
- `tmux pipe-pane` (引数なし) でパイプ停止
- ペイン ID オプションを `not logging` に設定
- ログファイルに `[timestamp] === Session Disconnected ===` を追記
- `gzip --force` でログ圧縮

---

## 5. ユーティリティ関数

### 5.1 `__fterm_debug` — デバッグ出力

- `FTERM_DEBUG` が `true` の場合のみ、stderr に `[DEBUG] {message}` を brblack 色で出力

### 5.2 `__fterm_format_duration` — 経過時間フォーマット

- 引数: 秒数 (整数)
- 出力: `{d}d {h}h{m}m{s}s` 形式 (日の後にスペース、他は区切りなし)
- 空引数は `0s`
- 0 でない最上位単位から表示 (例: 3665秒 → `1h1m5s`)

### 5.3 `__fssh_scp_extract_hosts` — SCP 引数からホスト抽出

- `-` で始まる引数 (オプション) はスキップ
- `:` を含む引数からホスト部分を抽出 (`:` 以前)
- 重複排除してユニークなホストリストを出力

### 5.4 `__fssh_ssh_is_dry_run` / `__fssh_scp_is_dry_run` — dry-run 検出

- SSH: `-G`, `-V`, `-Q`, `--help` のいずれかがあれば return 0
- SCP: `--help`, `-h` のいずれかがあれば return 0

### 5.5 `__fssh_ssh_status_splash` / `__fssh_scp_status_splash` — 接続バナー表示

- ASCII アートバナーと接続情報 (ホスト、タイムスタンプ、コマンド、ログファイル) を表示
- SSH 設定詳細とエージェント鍵情報を表示

---

## 6. 初期化 (`conf.d/fterm.fish`)

**Fisher プラグインとしてのイベントハンドラ:**

**初期化処理:**

1. 環境検出 (MSYS2 vs Unix)
2. デフォルト値設定 (未設定時のみ):
   - `FSSH_SSH_CONF_DIR`: SSH config ディレクトリ
   - `FTERM_LOG_DIR_PREFIX`: ログ保存先
   - `FTERM_SSH_WIN_GIT_DIR`: Git for Windows パス
3. MSYS2 の場合: 既知のパス (`/c/Windows/System32/OpenSSH`, `/c/Program Files/OpenSSH`) から Windows OpenSSH を自動検出
   - `__fssh_ssh_config` (Windows パス形式の config ファイル)
   - `ssh-add`, `ssh-keygen`, `git` のラッパー関数を作成

**インストールイベント:**

- 依存コマンドの存在確認: `ansifilter`, `awk`, `curl`, `find`, `fzf`, `gzip`, `rg`, `ssh`, `ssh-add`, `tmux`, `zcat`
- **注意:** `ssh-keygen` が依存リストに含まれていないが、`__fssh_check_identity_file` で使用される (漏れ)

**アンインストールイベント:**

- 全グローバル変数と関数を erase

---

## 7. 補完 (`completions/`)

- `fssh.fish`: ファイル補完を無効化 (`--no-files`)
- `ssh.fish`: `__fssh_get_hosts` からホスト補完 + 全 SSH オプションの補完定義
- `scp.fish`: `__fssh_get_hosts` からホスト補完 (`:` サフィックス付き) + 全 SCP オプションの補完定義

---

## 8. 確認済みの問題と仕様決定

### Q1: `__fssh_check_proxyjump` の戻り値 → **バグ (修正する)**

line 116: ProxyJump 先ホストの basic_config / identity_file エラーを検出しても常に `return 0` する。Rust 版ではエラー検出時に適切なステータスを返すよう修正する。

### Q2: `ssh.fish` の target_host 抽出 → **バグ (将来対応)**

`$argv[-1]` (最後の引数) をターゲットホストとして使用。`ssh -p 22 hostname command` のようにリモートコマンドを指定した場合に誤ったホスト名が取得される。Rust 版では SSH 引数をパースしてホスト名を正しく抽出する。

### Q3: `ssh.fish` の ssh-add 失敗時の挙動 → **条件付き制約 (Rust 版で変更)**

**現行の Fish 実装:** ssh-add 失敗時は一律 return 1 でブロック

**Rust 版での変更:**

- `-i` オプションが明示指定されていない場合: ssh-agent が必須 (失敗時はブロック)
- `-i` オプションで鍵を明示指定している場合: ssh-agent なしでも接続を許可

### Q4: `__fterm_format_duration` のフォーマット → **意図的 (そのまま)**

出力形式 `3d 23h30m49s` をそのまま Rust 版でも維持する。

### Q5: `ssh.fish` の戻り値 → **バグ (Rust 版で修正)**

SSH コマンドの終了ステータスを明示的にキャプチャしていない。`command "$ssh_cmd" $config_args $argv` の後にログ停止やキャッシュクリアが実行されるため、最後のコマンドのステータスが暗黙的に返る。Rust 版では SSH の終了ステータスを正しく返す。

### Q6: `__fssh_check_host_prefix` のパーツ数閾値

`parts_count < 2` (1 パーツ以下をスキップ)。2 パーツ以上のホストにはワイルドカードパターンが必要。現行仕様を維持。

---

## 9. アーキテクチャ方針まとめ

### tmux について

tmux は維持する。使用する tmux 機能:

- ペイン分割 (水平/垂直)
- ペインリサイズ
- ペインボーダーに `ssh:<hostname>` を表示
- `pipe-pane` によるセッションログ記録
- ウィンドウ/ペインオプションによる接続状態管理

### Rust 版で排除する外部依存

| 現行依存                   | 代替                                                |
| -------------------------- | --------------------------------------------------- |
| `ansifilter`               | `strip-ansi-escapes` crate                          |
| `awk` (タイムスタンプ付与) | Rust 内蔵ロジック (`fterm log-filter` サブコマンド) |
| Fish shell (ロジック)      | Rust バイナリ                                       |

### Rust 版で維持する外部依存

`ssh`, `ssh-add`, `ssh-keygen`, `scp`, `tmux`, `fzf`, `rg`, `gzip`, `find`, `zcat`
