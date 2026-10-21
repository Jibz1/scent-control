# The broker a desktop build is given, for when nothing is saved. Sourced by
# scripts/bundle-macos.sh; see .env.example. Works in the bash macOS ships (3.2).
#
# load_broker_env FILE
#   Exports SCENT_BROKER_URL, SCENT_BROKER_USERNAME and SCENT_ROUTE_ID from FILE, if it
#   exists, and nothing else. A value already in the environment wins, unless it is
#   empty. Lines are KEY=value, optionally after `export`, with Unix or Windows line
#   ends; blank lines and lines starting with # are skipped. A value in quotes is what
#   is between them; one without ends at a # after a space (an inline comment). A key
#   with PASSWORD in its name, in any case, is skipped with a one-line warning that
#   never shows its value, nor the key unless it is a plain name (else the line number):
#   the broker password lives in the Keychain, saved from Settings, and is never
#   compiled into the app.
#
# check_broker_url
#   Leaves SCENT_BROKER_URL out of the build, with a one-line warning that never shows
#   it, unless it is a wss:// address with no user name, password or whitespace in it.
#   The app would turn any other down, and a password in one would be compiled in.
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
  local file="$1" line key value number=0
  [[ -f "$file" ]] || return 0
  while IFS= read -r line || [[ -n "$line" ]]; do
    number=$((number + 1))
    line="$(_broker_env_trim "${line%$'\r'}")"
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
        case "$key" in
          [0-9]* | *[!A-Za-z0-9_]*) key="line $number of" ;;
          *) key="$key in" ;;
        esac
        echo "warning: ignoring $key $file: the broker password belongs in the Keychain (save it in Settings), never in .env." >&2
        continue
        ;;
      SCENT_BROKER_URL | SCENT_BROKER_USERNAME | SCENT_ROUTE_ID) ;;
      *) continue ;;
    esac
    value="$(_broker_env_trim "${line#*=}")"
    case "$value" in
      \"*) value="${value#\"}" && value="${value%%\"*}" ;;
      \'*) value="${value#\'}" && value="${value%%\'*}" ;;
      *) value="$(_broker_env_trim "${value%%[[:space:]]#*}")" ;;
    esac
    if [[ -z "${!key:-}" ]]; then
      export "$key=$value"
    fi
  done <"$file"
}

check_broker_url() {
  local url="${SCENT_BROKER_URL:-}"
  [[ -n "$url" ]] || return 0
  case "$url" in
    *@* | *[[:space:]]* | *[[:cntrl:]]*) ;;
    wss://?*) return 0 ;;
  esac
  unset SCENT_BROKER_URL
  echo "warning: ignoring SCENT_BROKER_URL: it isn't a wss:// address without a user name or password in it, so the app will ask for a broker in Settings." >&2
}

unset_passwords() {
  local name
  for name in $(compgen -e); do
    case "$name" in
      *[Pp][Aa][Ss][Ss][Ww][Oo][Rr][Dd]*) unset "$name" ;;
    esac
  done
}
