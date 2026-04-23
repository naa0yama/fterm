#!/usr/bin/env bash
set -euo pipefail

echo "Validating mounted files and directories..."

# List of expected mounted files and directories (optional)
EXPECTED_MOUNTS=(
	"$HOME/.claude.json"
	"$HOME/.claude/"
)

validation_failed=false

# Check each expected mount
for mount_path in "${EXPECTED_MOUNTS[@]}"; do
	if [[ ! -e "$mount_path" ]]; then
		echo -e "\e[33mWARNING: Mount target not found: $mount_path\e[0m"
		validation_failed=true
	else
		echo "✓ Mount validated: $mount_path"
	fi
done

if [ "$validation_failed" = true ]; then
	echo ""
	echo -e "\e[33m================================================================================\e[0m"
	echo -e "\e[33m>>>                                WARNING                                   <<<\e[0m"
	echo -e "\e[33m>>>\t一部のマウントが見つかりませんが、開発は続行可能です。\e[0m"
	echo -e "\e[33m>>>\t必要に応じて devcontainer.json の mounts を確認してください。\e[0m"
	echo -e "\e[33m>>>\ttarget にはマウント先の full path が含まれるためユーザー名を変更した\e[0m"
	echo -e "\e[33m>>>\t場合修正が必要です。\e[0m"
	echo -e "\e[33m================================================================================\e[0m"
	echo ""
else
	echo "All mounts validated successfully!"
fi

chmod +x .githooks/*
mise trust /app/mise.toml
mise install

# Install OpenObserve
echo "Installing OpenObserve..."
mise run o2:install

# 1. authorized_keys setup
rm -rf ~/.ssh/id_ed25519 ~/.ssh/id_ed25519.pub
mkdir -p ~/.ssh

chmod 0700 ~/.ssh
rm -f ~/.ssh/id_ed25519 ~/.ssh/id_ed25519.pub
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519 -N "" -C "" -q

touch ~/.ssh/authorized_keys
chmod 0600 ~/.ssh/authorized_keys
cat ~/.ssh/id_ed25519.pub > ~/.ssh/authorized_keys

mv -v ~/.ssh/id_ed25519			/app/tests/.ssh/conf.d/keys/private/id_ed25519.pem
mv -v ~/.ssh/id_ed25519.pub		/app/tests/.ssh/conf.d/keys/public/id_ed25519.pem
rm -rf ~/.ssh/id_ed25519		~/.ssh/id_ed25519.pub

# 2. Link test SSH conf.d into ~/.ssh so OpenSSH resolves Include paths correctly
#    ssh -F /app/tests/.ssh/config resolves relative Includes from ~/.ssh/,
#    so ~/.ssh/conf.d must exist and point to the test config directory.
ln -sf /app/tests/.ssh/conf.d ~/.ssh/conf.d

# 3. sshd privilege separation directory
sudo mkdir -p /run/sshd

# 4. Generate host keys if missing
sudo ssh-keygen -A

# 5. Start sshd
sudo /usr/sbin/sshd
