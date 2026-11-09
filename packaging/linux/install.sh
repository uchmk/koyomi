#!/usr/bin/env bash
# Install or update the koyomi engine as a systemd service on Debian, or remove
# it with --uninstall. Safe to run again. Run as root, from the unpacked folder.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
home=/var/lib/koyomi
conf_dir=$home/.config/uchmk/koyomi
conf=$conf_dir/koyomi.toml
cred_dir=/etc/koyomi/credentials
unit=/etc/systemd/system/koyomi.service

usage() {
  cat <<'USAGE'
使い方: sudo ./install.sh [--port <n>]
        sudo ./install.sh --uninstall

  (引数なし)   koyomi のエンジンを入れる。入っていれば更新する。
  --port <n>   待ち受けるポート (既定 8790)。最初に koyomi.toml を書くときだけ効く。
  --uninstall  サービスと実行ファイルを消す。データと設定と利用者は残す。
  -h           この説明。
USAGE
}

die() { echo "エラー: $*" >&2; exit 1; }

port=8790
port_given=0
mode=install
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --uninstall) mode=uninstall; shift ;;
    --port)
      [ "$#" -ge 2 ] || die "--port には番号が要ります。"
      port=$2; port_given=1; shift 2 ;;
    *) usage >&2; exit 2 ;;
  esac
done

if [ "$port_given" -eq 1 ]; then
  # No leading zero: TOML rejects `port = 08790`.
  [[ "$port" =~ ^[1-9][0-9]*$ ]] && [ "$port" -le 65535 ] \
    || die "--port は 1 から 65535 の数にしてください。"
fi

[ "$(id -u)" -eq 0 ] || die "root で実行してください (sudo ./install.sh)。"

# Run a command as the koyomi user with a clean XDG environment. Everything
# under /var/lib/koyomi is written and read through this: that tree is
# writable by koyomi, so root must not create, chown or read files in it.
as_koyomi() {
  runuser -u koyomi -- env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_RUNTIME_DIR HOME="$home" "$@"
}

# Read the port from the [http] table of the current koyomi.toml. Prints it only
# when the whole value is digits (underscores allowed, as in TOML: 8_790).
conf_port() {
  as_koyomi awk '
    /^[[:space:]]*\[/ { in_http = ($0 ~ /^[[:space:]]*\[http\][[:space:]]*(#.*)?$/); next }
    in_http && /^[[:space:]]*port[[:space:]]*=/ {
      v = $0
      sub(/^[^=]*=/, "", v); sub(/#.*$/, "", v)
      gsub(/[[:space:]]/, "", v); gsub(/_/, "", v)
      if (v ~ /^[0-9]+$/) print v
      exit
    }
  ' "$conf"
}

conf_has_lan() {
  as_koyomi awk '
    /^[[:space:]]*\[/ { in_http = ($0 ~ /^[[:space:]]*\[http\][[:space:]]*(#.*)?$/); next }
    in_http && /^[[:space:]]*lan[[:space:]]*=[[:space:]]*true/ { found = 1 }
    END { exit found ? 0 : 1 }
  ' "$conf"
}

if [ "$mode" = uninstall ]; then
  if [ -f "$unit" ]; then
    systemctl disable --now koyomi || die "koyomi を止められませんでした。何も消していません。"
  fi
  rm -f "$unit" /usr/local/bin/koyomi /usr/local/sbin/koyomi-secret
  systemctl daemon-reload || true
  echo "koyomi のサービスと実行ファイルを消しました。"
  echo "次は残してあります。要らなければ自分で消してください (ここでは実行しません)。"
  echo "  $home  (設定と状態)"
  echo "  /etc/koyomi  (token などの秘密)"
  echo "  利用者 koyomi"
  echo "消すとき:"
  echo "  sudo rm -rf $home /etc/koyomi && sudo userdel koyomi"
  exit 0
fi

# 1. Check the environment and the package.
[ -d /run/systemd/system ] || die "systemd で動いていません。この手順は systemd が要ります。"
for f in koyomi koyomi.service koyomi-secret; do
  [ -f "$here/$f" ] || die "$here に $f がありません。展開したフォルダーで実行してください。"
done
if ! ver_out=$("$here/koyomi" --version 2>&1); then
  echo "$ver_out" >&2
  die "同梱の koyomi が動きません。CPU の種類 (x86_64 / aarch64) が違うか、OS が古すぎる可能性があります。"
fi
echo "実行ファイル: $ver_out"

# 2. The service user.
if ! id koyomi >/dev/null 2>&1; then
  useradd --system --user-group --home-dir "$home" --create-home --shell /usr/sbin/nologin koyomi
  echo "利用者 koyomi を作りました。"
fi
install -d -m 0750 -o koyomi -g koyomi "$home"

# 3. The admin token, only when there is none yet.
install -d -m 0700 -o root -g root /etc/koyomi "$cred_dir"
if [ ! -e "$cred_dir/http-admin" ]; then
  tmp=$(mktemp "$cred_dir/.tmp.XXXXXX")
  trap 'rm -f "$tmp"' EXIT
  od -An -tx1 -N32 /dev/urandom | tr -d ' \n' > "$tmp"
  chown root:root "$tmp"
  chmod 0600 "$tmp"
  mv -f "$tmp" "$cred_dir/http-admin"
  trap - EXIT
  echo "admin の token を作りました。"
else
  echo "admin の token は既にあります。変えません。"
fi

# 4. Check that this systemd can read a credential folder. Done before anything
#    running is touched, so a failure leaves a working koyomi alone.
if ! systemd-run --wait --pipe --quiet -p LoadCredential=koyomi:"$cred_dir" \
    /bin/sh -c 'test -r "$CREDENTIALS_DIRECTORY/koyomi_http-admin"'; then
  die "この systemd は credential のフォルダーを読めません。Debian 12 以降が要ります。koyomi の起動も更新もしていません。"
fi

# 5. Stop the unit if it is installed (systemctl waits until it has stopped;
#    `koyomi engine stop` does not). Not tied to is-active, so that a unit
#    waiting in `activating (auto-restart)` is stopped too.
if [ -f "$unit" ]; then
  echo "koyomi を止めます。動いているジョブは止まります。"
  systemctl stop koyomi
fi
trap 'echo "途中で失敗しました。koyomi は動いていません。原因を直してから install.sh をもう一度実行してください。" >&2' ERR

# 6. Put the files in place.
install -m 0755 -o root -g root "$here/koyomi" /usr/local/bin/koyomi
install -d -m 0755 /usr/local/sbin
install -m 0755 -o root -g root "$here/koyomi-secret" /usr/local/sbin/koyomi-secret
install -m 0644 -o root -g root "$here/koyomi.service" "$unit"
systemctl daemon-reload

# 7. First-time settings, written as koyomi (never by root inside its tree).
if ! as_koyomi test -e "$conf"; then
  as_koyomi /bin/sh -c '
    set -e
    umask 027
    mkdir -p "$1"
    tmp=$(mktemp "$1/.tmp.XXXXXX")
    trap "rm -f \"\$tmp\"" EXIT
    cat > "$tmp"
    chmod 0640 "$tmp"
    mv -f "$tmp" "$2"
    trap - EXIT
  ' sh "$conf_dir" "$conf" <<TOML
[http]
port = $port
lan = true

[[http.user]]
name = "admin"
role = "admin"
token = "secret:http-admin"
TOML
  echo "設定を書きました: $conf (port $port)"
else
  echo "設定は既にあります。触りません: $conf"
  if [ "$port_given" -eq 1 ]; then
    echo "  --port は初回だけ効きます。今の koyomi.toml の port を使います。"
  fi
  if ! conf_has_lan; then
    echo "  注意: [http] に lan = true がありません。このままでは Windows からつなげません。"
  fi
fi

# 8. Read the fingerprint before the start (a failure here does not stop the install).
fp_help='sudo runuser -u koyomi -- env -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_RUNTIME_DIR HOME=/var/lib/koyomi /usr/local/bin/koyomi http fingerprint'
if fp=$(as_koyomi /usr/local/bin/koyomi http fingerprint); then
  # The command adds a note about [[remote]] below the fingerprint.
  fp=${fp%%$'\n'*}
else
  echo "指紋を読めませんでした。" >&2
  fp=
fi

# 9. Start.
systemctl enable --now koyomi || true
sleep 3
if ! systemctl is-active --quiet koyomi; then
  echo "koyomi が起動していません。次で理由を見てください:" >&2
  echo "  journalctl -u koyomi -n 30 --no-pager" >&2
  exit 1
fi
trap - ERR

# 10. Summary.
cur_port=$(conf_port || true)
echo
echo "入れました。"
echo
if [ -n "$cur_port" ]; then
  echo "つなぐ URL:"
  for a in $(hostname -I 2>/dev/null || true); do
    case "$a" in
      *:*) echo "  https://[$a]:$cur_port" ;;
      *) echo "  https://$a:$cur_port" ;;
    esac
  done
else
  echo "port を読めませんでした。$conf の [http] を見てください。"
fi
echo
if [ -n "$fp" ]; then
  echo "指紋 (窓に出る指紋と同じか確かめてください):"
  echo "  $fp"
else
  echo "指紋は次で読み直せます:"
  echo "  $fp_help"
fi
echo
echo "admin の token は次で読めます (値はここには出しません):"
echo "  sudo cat $cred_dir/http-admin"
echo
echo "Windows の PC からこのポート (${cur_port:-koyomi.toml の port}) に TCP で届く必要があります (ファイアウォールを確かめてください)。"
tz=$(timedatectl show -p Timezone --value 2>/dev/null || true)
[ -n "$tz" ] || tz=$(cat /etc/timezone 2>/dev/null || true)
echo "この機械の時刻帯: ${tz:-不明}  (変えません。窓の PC と同じ時刻帯で使ってください)"
echo
echo "ここでの操作の例:"
echo "  sudo -H -u koyomi koyomi status"
