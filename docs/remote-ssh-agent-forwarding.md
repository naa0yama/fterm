# Remote SSH + Agent Forwarding 環境での SSH_AUTH_SOCK 管理

## 問題

SSH Agent Forwarding を使ってリモートサーバーに接続し、tmux 内で作業する場合、
`SSH_AUTH_SOCK` が陳腐化する問題がある。

### 仕組み

1. SSH 接続時、sshd は一時的なソケットパス（例: `/tmp/ssh-XXXX/agent.NNN`）を `SSH_AUTH_SOCK` に設定する
2. tmux セッションにアタッチすると、`update-environment` により一時パスが tmux のグローバル環境に伝搬する
3. SSH 接続が切断されると一時ソケットは削除されるが、tmux 内の `SSH_AUTH_SOCK` は古いパスを指したまま残る
4. 再接続時に新しい一時パスが生成されるが、既存の tmux ペインには反映されない

## 解決策: 3 層アプローチ

| 層 | ファイル           | 役割                                                                 |
| -- | ------------------ | -------------------------------------------------------------------- |
| 1  | `~/.ssh/rc`        | `ln -sf "$SSH_AUTH_SOCK" ~/.ssh/agent.sock` で安定シンボリンクを作成 |
| 2  | シェルプロファイル | `SSH_AUTH_SOCK=~/.ssh/agent.sock` を export                          |
| 3  | `.tmux.conf`       | `update-environment` から `SSH_AUTH_SOCK` を除外                     |

### 層 1: `~/.ssh/rc`

SSH 接続のたびに sshd が実行する。一時ソケットへのシンボリンクを固定パスに作成する。

```bash
#!/usr/bin/env bash
if [ -n "$SSH_AUTH_SOCK" ] && [ "$SSH_AUTH_SOCK" != "$HOME/.ssh/agent.sock" ]; then
    ln -sf "$SSH_AUTH_SOCK" "$HOME/.ssh/agent.sock"
fi
```

### 層 2: シェルプロファイル

#### bash (`~/.bash_profile` or `~/.profile`)

```bash
if [ -S "$HOME/.ssh/agent.sock" ]; then
    export SSH_AUTH_SOCK="$HOME/.ssh/agent.sock"
fi
```

#### fish (`~/.config/fish/config.fish`)

fterm の `skel/.config/fish/config.fish` に含まれている:

```fish
if not builtin set --query MSYSTEM
    if builtin test -S "$HOME/.ssh/agent.sock"
        builtin set --global --export SSH_AUTH_SOCK "$HOME/.ssh/agent.sock"
    end
end
```

### 層 3: `.tmux.conf`

fterm の `skel/.tmux.conf` に含まれている。tmux のデフォルト `update-environment` から
`SSH_AUTH_SOCK` を除外する:

```tmux
# ------------------------------------------------------------------------------
# SSH Agent (Agent Forwarding)
# ------------------------------------------------------------------------------
# SSH Agent Forward 利用時、tmux が一時ソケットパスで SSH_AUTH_SOCK を上書きするのを防ぐ。
# ~/.ssh/agent.sock は ~/.ssh/rc によって常に最新のソケットを指すシンボリンクとして維持され、
# シェルプロファイルで SSH_AUTH_SOCK に設定される。tmux がこの値を上書きしないようにする。
if-shell "test -z \"$MSYSTEM\"" {
    set-option -g update-environment "DISPLAY KRB5CCNAME SSH_ASKPASS SSH_AGENT_PID SSH_CONNECTION WINDOWID XAUTHORITY"
}
```

## fterm の立場

fterm は `SSH_AUTH_SOCK` を **信頼し、操作しない**。

以前のバージョンでは `tmux show-environment SSH_AUTH_SOCK` を読み取って環境変数を
書き換えていたが、これは環境側の設定を壊す方向に作用するため削除した。
SSH_AUTH_SOCK の管理はシェルプロファイルと tmux の設定に委ねる。

## 動作確認

### SSH_AUTH_SOCK が正しく設定されているか

```bash
# tmux 起動前
echo $SSH_AUTH_SOCK
# → ~/.ssh/agent.sock

# tmux 内
echo $SSH_AUTH_SOCK
# → ~/.ssh/agent.sock

# tmux のグローバル環境
tmux show-environment SSH_AUTH_SOCK
# → 未設定 または ~/.ssh/agent.sock
```

### agent.sock シンボリンクが有効か

```bash
ls -la ~/.ssh/agent.sock
# → lrwxrwxrwx ... ~/.ssh/agent.sock -> /tmp/ssh-XXXX/agent.NNN

ssh-add -l
# → 鍵一覧が表示されること
```

## MSYS2 環境について

Windows (MSYS2) 環境では Windows OpenSSH agent が別の仕組み（名前付きパイプ）で
動作するため、上記の設定は適用されない。`if-shell "test -z \"$MSYSTEM\""` および
`if not builtin set --query MSYSTEM` で MSYS2 環境を除外している。
