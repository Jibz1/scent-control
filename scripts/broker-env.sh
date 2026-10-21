# The broker a desktop build is given, for when nothing is saved. Sourced by
# scripts/bundle-macos.sh; see .env.example. Works in the bash macOS ships (3.2).
#
# load_broker_env FILE
#   Exports SCENT_BROKER_URL, SCENT_BROKER_USERNAME and SCENT_ROUTE_ID from FILE, if it
#   exists, and nothing else. A value already in the environment wins. Lines are
#   KEY=value, optionally after `export` and with the value in quotes; blank lines and
#   lines starting with # are skipped. A key with PASSWORD in its name, in any case, is
#   skipped with a one-line warning that names the key and never shows its value: the
#   broker password lives in the Keychain, saved from Settings, and is never compiled
#   into the app.
#
# unset_passwords
#   Removes every variable with PASSWORD in its name from this shell's environment, so
#   that none reaches the build from the shell either.

_broker_env_trim() {
  local text="$1"
  text="${text#"${text%%[![:space:]]*}"}"
  text="${text%"${text##*[![:space:]]}"}"
  printf '%s' "$text"
}

load_broker_env() {
  local file="$1" line key value
  [[ -f "$file" ]] || return 0
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="$(_broker_env_trim "$line")"
    case "$line" in
      "" | \#*) continue ;;
    esac
    case "$line" in
      export[[:space:]]*) line="$(_broker_env_trim "${line#export}")" ;;
    esac
    case "$line" in
      *=*) ;;
      *) continue ;;
    esac
    key="$(_broker_env_trim "${line%%=*}")"
    case "$key" in
      *[Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd]*)
        echo "warning: ignoring $key in $file: the broker password belongs in the Keychain (save it in Settings), never in .env." >&2
        continue
        ;;
      SCENT_BROKER_URL | SCENT_BROKER_USERNAME | SCENT_ROUTE_ID) ;;
      *) continue ;;
    esac
    value="$(_broker_env_trim "${line#*=}")"
    case "$value" in
      \"*\") value="${value#\"}" && value="${value%\"}" ;;
      \'*\') value="${value#\'}" && value="${value%\'}" ;;
    esac
    if ! printenv "$key" >/dev/null; then
      export "$key=$value"
    fi
  done <"$file"
}

unset_passwords() {
  local name
  for name in $(compgen -e); do
    case "$name" in
      *[Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd]*) unset "$name" ;;
    esac
  done
}
